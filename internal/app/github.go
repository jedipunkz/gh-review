package app

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"sort"
	"strings"
	"time"
)

type pullRequest struct {
	Repository string
	Number     int
	Title      string
	URL        string
	Author     string
	UpdatedAt  time.Time
	Request    string
}

type team struct {
	Organization string
	Slug         string
}

type issueSearchResponse struct {
	Items []struct {
		Number        int       `json:"number"`
		Title         string    `json:"title"`
		URL           string    `json:"html_url"`
		RepositoryURL string    `json:"repository_url"`
		UpdatedAt     time.Time `json:"updated_at"`
		User          struct {
			Login string `json:"login"`
		} `json:"user"`
	} `json:"items"`
}

func loadReviewRequests(ctx context.Context) ([]pullRequest, error) {
	queries := []struct {
		label string
		q     string
	}{
		{
			label: "@me",
			q:     "is:pr is:open archived:false review-requested:@me",
		},
	}

	teams, err := loadTeams(ctx)
	if err == nil {
		for _, t := range teams {
			if t.Organization == "" || t.Slug == "" {
				continue
			}
			name := t.Organization + "/" + t.Slug
			queries = append(queries, struct {
				label string
				q     string
			}{
				label: name,
				q:     "is:pr is:open archived:false team-review-requested:" + name,
			})
		}
	}

	byURL := make(map[string]pullRequest)
	var errs []error
	for _, query := range queries {
		prs, err := searchPRs(ctx, query.q, query.label)
		if err != nil {
			errs = append(errs, fmt.Errorf("%s: %w", query.label, err))
			continue
		}
		for _, pr := range prs {
			if existing, ok := byURL[pr.URL]; ok {
				if !strings.Contains(existing.Request, pr.Request) {
					existing.Request += ", " + pr.Request
				}
				byURL[pr.URL] = existing
				continue
			}
			byURL[pr.URL] = pr
		}
	}

	if len(byURL) == 0 && len(errs) > 0 {
		return nil, errors.Join(errs...)
	}

	prs := make([]pullRequest, 0, len(byURL))
	for _, pr := range byURL {
		prs = append(prs, pr)
	}
	sort.Slice(prs, func(i, j int) bool {
		return prs[i].UpdatedAt.After(prs[j].UpdatedAt)
	})
	return prs, nil
}

func loadTeams(ctx context.Context) ([]team, error) {
	out, err := runGH(ctx, "api", "user/teams", "--paginate", "--jq", `.[] | [.organization.login, .slug] | @tsv`)
	if err != nil {
		return nil, err
	}

	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	teams := make([]team, 0, len(lines))
	for _, line := range lines {
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "\t", 2)
		if len(parts) != 2 {
			continue
		}
		teams = append(teams, team{
			Organization: parts[0],
			Slug:         parts[1],
		})
	}
	return teams, nil
}

func searchPRs(ctx context.Context, query, label string) ([]pullRequest, error) {
	out, err := runGH(ctx, "api", "--method", "GET", "/search/issues", "-f", "q="+query, "-f", "per_page=100")
	if err != nil {
		return nil, err
	}

	var res issueSearchResponse
	if err := json.Unmarshal(out, &res); err != nil {
		return nil, err
	}

	prs := make([]pullRequest, 0, len(res.Items))
	for _, item := range res.Items {
		prs = append(prs, pullRequest{
			Repository: repositoryName(item.RepositoryURL),
			Number:     item.Number,
			Title:      item.Title,
			URL:        item.URL,
			Author:     item.User.Login,
			UpdatedAt:  item.UpdatedAt,
			Request:    label,
		})
	}
	return prs, nil
}

func loadDiff(ctx context.Context, pr pullRequest) (string, error) {
	out, err := runGH(ctx, "pr", "diff", pr.URL, "--color=never")
	if err != nil {
		return "", err
	}
	return string(out), nil
}

func approvePR(ctx context.Context, pr pullRequest) error {
	_, err := runGH(ctx, "pr", "review", pr.URL, "--approve")
	return err
}

func runGH(ctx context.Context, args ...string) ([]byte, error) {
	cmd := exec.CommandContext(ctx, "gh", args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		message := strings.TrimSpace(stderr.String())
		if message == "" {
			message = err.Error()
		}
		return nil, fmt.Errorf("gh %s: %s", strings.Join(args, " "), message)
	}
	return out, nil
}

func repositoryName(apiURL string) string {
	const marker = "/repos/"
	i := strings.Index(apiURL, marker)
	if i == -1 {
		return apiURL
	}
	return apiURL[i+len(marker):]
}
