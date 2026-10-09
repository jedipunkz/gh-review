use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use gh_review::app::{Effect, Model, Msg};
use gh_review::cache::DetailCache;
use gh_review::gh::Gh;
use gh_review::prefetch::Prefetcher;

struct Runtime {
    tx: mpsc::UnboundedSender<Msg>,
    inflight_load: Option<tokio::task::JoinHandle<()>>,
    cache: Arc<DetailCache>,
    prefetcher: Arc<Prefetcher>,
}

impl Runtime {
    fn execute(&mut self, g: Gh, effect: Effect) {
        let tx = self.tx.clone();
        match effect {
            Effect::LoadPRs => {
                let g2 = g.clone();
                tokio::spawn(async move {
                    let result = gh_review::gh::load_review_requests(&g2)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::PRList(result));
                });
            }
            Effect::LoadHistory { state } => {
                let g2 = g.clone();
                tokio::spawn(async move {
                    let result = gh_review::gh::load_history(&g2, state)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::HistoryList { state, result });
                });
            }
            Effect::CheckUpdates {
                prev_sig,
                prev_count,
            } => {
                let g2 = g.clone();
                tokio::spawn(async move {
                    let result = gh_review::gh::load_review_requests(&g2)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::UpdateCheck {
                        prev_sig,
                        prev_count,
                        result,
                    });
                });
            }
            Effect::LoadDiff { pr } => {
                if let Some(handle) = self.inflight_load.take() {
                    handle.abort();
                }
                let g2 = g.clone();
                let cache = self.cache.clone();
                self.inflight_load = Some(tokio::spawn(async move {
                    let result = gh_review::gh::load_detail_and_diff(&g2, &pr.clone()).await;
                    if let Ok((ref detail, ref diff)) = result {
                        let key =
                            gh_review::gh::cache_key_of_updated(&pr.url, detail.base.updated_at);
                        cache.put(
                            &key,
                            gh_review::cache::CacheEntry {
                                detail: detail.clone(),
                                diff: diff.clone(),
                            },
                        );
                    }
                    let _ = tx.send(Msg::DiffDone(Box::new(gh_review::app::DiffDoneMsg {
                        pr,
                        result: result.map_err(|e| e.to_string()),
                    })));
                }));
            }
            Effect::Debounce { seq, url } => {
                tokio::spawn(async move {
                    tokio::time::sleep(gh_review::app::DETAIL_LOAD_DEBOUNCE).await;
                    let _ = tx.send(Msg::DebounceFire { seq, url });
                });
            }
            Effect::Approve { pr } => {
                let g2 = g.clone();
                tokio::spawn(async move {
                    let err = gh_review::gh::approve_pr(&g2, &pr)
                        .await
                        .err()
                        .map(|e| e.to_string());
                    let _ = tx.send(Msg::ApproveDone { pr, err });
                });
            }
            Effect::CopyURL { pr } => {
                tokio::spawn(async move {
                    let err = copy_url(&pr.url).await.err().map(|e| e.to_string());
                    let _ = tx.send(Msg::CopyDone { pr, err });
                });
            }
            Effect::PlaySound => {
                tokio::spawn(async move {
                    play_notify_sound().await;
                });
            }
            Effect::DismissPopup { id } => {
                tokio::spawn(async move {
                    tokio::time::sleep(gh_review::app::POPUP_DISMISS_DELAY).await;
                    let _ = tx.send(Msg::PopupDismiss { id });
                });
            }
            Effect::PrefetchHint { prs } => {
                self.prefetcher.spawn(&g, prs);
            }
        }
    }
}

async fn copy_url(url: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut child = Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(url.as_bytes()).await?;
        stdin.flush().await?;
        drop(stdin);
    }
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(status) => match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(std::io::Error::other(format!("pbcopy failed: {s}"))),
            Err(e) => Err(e),
        },
        Err(_) => Err(std::io::Error::other("pbcopy timed out")),
    }
}

async fn play_notify_sound() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let _ = tokio::process::Command::new("afplay")
        .arg("/System/Library/Sounds/Pop.aiff")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn spawn_key_reader(tx: mpsc::UnboundedSender<Msg>) {
    std::thread::spawn(move || loop {
        match crossterm::event::read() {
            Ok(crossterm::event::Event::Key(key)) => {
                if tx.send(Msg::Key(key)).is_err() {
                    break;
                }
            }
            Ok(crossterm::event::Event::Resize(w, h)) => {
                if tx.send(Msg::Resize(w, h)).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    });
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = gh_review::theme::config_path() {
        gh_review::theme::set_active(gh_review::theme::load_from(&path)?);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let mut terminal = ratatui::init();

    let result = rt.block_on(async {
        crossterm::terminal::enable_raw_mode().map_err(|e| e.to_string())?;

        let g = Gh::new();
        let cache = Arc::new(DetailCache::new());
        let prefetcher = Arc::new(Prefetcher::new(cache.clone()));
        let mut model = Model::new(cache.clone(), prefetcher.clone());

        let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
        let mut runtime = Runtime {
            tx: tx.clone(),
            inflight_load: None,
            cache,
            prefetcher,
        };

        spawn_key_reader(tx.clone());

        let tick_tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(gh_review::app::UPDATE_CHECK_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if tick_tx.send(Msg::UpdateCheckTick).is_err() {
                    break;
                }
            }
        });

        let spin_tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                interval.tick().await;
                if spin_tx.send(Msg::SpinnerTick).is_err() {
                    break;
                }
            }
        });

        let size = terminal.size().map_err(|e| e.to_string())?;
        let _ = tx.send(Msg::Resize(size.width, size.height));
        runtime.execute(Gh::new(), Effect::LoadPRs);

        let run_result: Result<(), String> = async {
            loop {
                terminal
                    .draw(|f| gh_review::app::render(&mut model, f))
                    .map_err(|e| e.to_string())?;

                let msg = match rx.recv().await {
                    Some(m) => m,
                    None => break,
                };

                for effect in model.update(msg) {
                    runtime.execute(g.clone(), effect);
                }
                if model.quit {
                    break;
                }
            }
            Ok(())
        }
        .await;

        run_result
    });

    ratatui::restore();
    crossterm::terminal::disable_raw_mode()?;
    result.map_err(|e| e.into())
}

fn main() {
    use crossterm::tty::IsTty;
    if !std::io::stdin().is_tty() {
        eprintln!("gh review: stdin is not a terminal");
        std::process::exit(1);
    }
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
