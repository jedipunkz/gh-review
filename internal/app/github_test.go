package app

import (
	"errors"
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
