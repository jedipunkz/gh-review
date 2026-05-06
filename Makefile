.PHONY: build test

build:
	go build -o gh-review ./cmd/gh-review

test:
	go test ./...
