package app

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"testing"
	"time"
)

func TestIsPRDiffTooLargeError(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want bool
	}{
		{
			name: "nil",
			err:  nil,
			want: false,
		},
		{
			name: "pull request diff too large",
			err:  errors.New("gh pr diff https://github.com/example/repo/pull/1 --color=always: could not find pull request diff: HTTP 406: Sorry, the diff exceeded the maximum number of files (300). PullRequest.diff too_large"),
			want: true,
		},
		{
			name: "maximum files message",
			err:  errors.New("HTTP 406: Sorry, the diff exceeded the maximum number of files (300). Consider using 'List pull requests files' API"),
			want: true,
		},
		{
			name: "unrelated gh error",
			err:  errors.New("gh pr diff: HTTP 404: Not Found"),
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := isPRDiffTooLargeError(tt.err); got != tt.want {
				t.Fatalf("isPRDiffTooLargeError() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestParseSearchPRsResponseIncludesReviewDecision(t *testing.T) {
	out := []byte(`{
		"data": {
			"search": {
				"pageInfo": {
					"hasNextPage": true,
					"endCursor": "cursor-1"
				},
				"nodes": [
					{
						"number": 42,
						"title": "Add review list status",
						"url": "https://github.com/owner/repo/pull/42",
						"updatedAt": "2026-05-13T01:02:03Z",
						"reviewDecision": "APPROVED",
						"repository": {
							"nameWithOwner": "owner/repo"
						},
						"author": {
							"login": "octocat"
						}
					}
				]
			}
		}
	}`)

	page, err := parseSearchPRsResponse(out, "@me")
	if err != nil {
		t.Fatal(err)
	}
	if !page.hasNextPage {
		t.Fatal("hasNextPage = false, want true")
	}
	if page.endCursor != "cursor-1" {
		t.Fatalf("endCursor = %q, want cursor-1", page.endCursor)
	}
	if len(page.prs) != 1 {
		t.Fatalf("len(prs) = %d, want 1", len(page.prs))
	}

	pr := page.prs[0]
	if pr.Repository != "owner/repo" {
		t.Fatalf("Repository = %q, want owner/repo", pr.Repository)
	}
	if pr.ReviewDecision != "APPROVED" {
		t.Fatalf("ReviewDecision = %q, want APPROVED", pr.ReviewDecision)
	}
	wantUpdatedAt := time.Date(2026, 5, 13, 1, 2, 3, 0, time.UTC)
	if !pr.UpdatedAt.Equal(wantUpdatedAt) {
		t.Fatalf("UpdatedAt = %s, want %s", pr.UpdatedAt, wantUpdatedAt)
	}
}

func TestLoadTeamsParsesTSVAndSkipsMalformedLines(t *testing.T) {
	withRunGH(t, func(ctx context.Context, args ...string) ([]byte, error) {
		wantArgs := []string{"api", "user/teams", "--paginate", "--jq", `.[] | [.organization.login, .slug] | @tsv`}
		if !reflect.DeepEqual(args, wantArgs) {
			t.Fatalf("args = %#v, want %#v", args, wantArgs)
		}
		return []byte("owner\tcore\nmalformed\n\tmissing-org\nowner\t\nplatform\treviewers\n"), nil
	})

	got, err := loadTeams(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	want := []team{
		{Organization: "owner", Slug: "core"},
		{Organization: "", Slug: "missing-org"},
		{Organization: "owner", Slug: ""},
		{Organization: "platform", Slug: "reviewers"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("teams = %#v, want %#v", got, want)
	}
}

func TestLoadPRDetailMergesGhResponseWithListPR(t *testing.T) {
	base := pullRequest{
		Repository:     "owner/repo",
		Number:         1,
		Title:          "from search",
		URL:            "https://github.com/owner/repo/pull/1",
		Author:         "search-author",
		UpdatedAt:      time.Date(2026, 5, 13, 1, 2, 3, 0, time.UTC),
		Request:        "@me",
		ReviewDecision: "REVIEW_REQUIRED",
	}
	withRunGH(t, func(ctx context.Context, args ...string) ([]byte, error) {
		if len(args) != 5 || args[0] != "pr" || args[1] != "view" || args[2] != base.URL || args[3] != "--json" {
			t.Fatalf("unexpected args: %#v", args)
		}
		if !strings.Contains(args[4], "mergeStateStatus") || !strings.Contains(args[4], "labels") {
			t.Fatalf("json fields missing detail fields: %q", args[4])
		}
		return []byte(`{
			"number": 42,
			"title": "from detail",
			"url": "https://github.com/owner/repo/pull/42",
			"body": "body text",
			"createdAt": "2026-05-12T01:02:03Z",
			"updatedAt": "2026-05-14T04:05:06Z",
			"baseRefName": "main",
			"headRefName": "feature",
			"reviewDecision": "APPROVED",
			"mergeStateStatus": "CLEAN",
			"additions": 10,
			"deletions": 2,
			"changedFiles": 3,
			"author": {"login": "detail-author"},
			"labels": [{"name": "bug"}, {"name": ""}, {"name": "ui"}]
		}`), nil
	})

	got, err := loadPRDetail(context.Background(), base)
	if err != nil {
		t.Fatal(err)
	}
	if got.Repository != base.Repository || got.Request != base.Request {
		t.Fatalf("list fields were not preserved: %+v", got)
	}
	if got.Number != 42 || got.Title != "from detail" || got.URL != "https://github.com/owner/repo/pull/42" {
		t.Fatalf("detail identity fields not applied: %+v", got)
	}
	if got.Author != "detail-author" || got.ReviewDecision != "APPROVED" || got.MergeStateStatus != "CLEAN" {
		t.Fatalf("detail metadata not applied: %+v", got)
	}
	if got.BaseRefName != "main" || got.HeadRefName != "feature" || got.Additions != 10 || got.Deletions != 2 || got.ChangedFiles != 3 {
		t.Fatalf("diff stats or branch fields not applied: %+v", got)
	}
	if !reflect.DeepEqual(got.Labels, []string{"bug", "ui"}) {
		t.Fatalf("labels = %#v, want bug/ui", got.Labels)
	}
	wantUpdatedAt := time.Date(2026, 5, 14, 4, 5, 6, 0, time.UTC)
	if !got.UpdatedAt.Equal(wantUpdatedAt) {
		t.Fatalf("UpdatedAt = %s, want %s", got.UpdatedAt, wantUpdatedAt)
	}
}

func withRunGH(t *testing.T, fn func(context.Context, ...string) ([]byte, error)) {
	t.Helper()
	orig := runGH
	runGH = fn
	t.Cleanup(func() { runGH = orig })
}
