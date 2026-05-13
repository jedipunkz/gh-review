package app

import (
	"errors"
	"testing"
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
