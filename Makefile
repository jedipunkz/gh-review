.PHONY: build test

build:
	go build -o gh-review-cli ./cmd/gh-review-cli

test:
	go test ./...
