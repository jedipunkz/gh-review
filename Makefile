GH_EXTENSIONS_DIR ?= $(HOME)/.local/share/gh/extensions
GH_EXTENSION_NAME ?= gh-review

.PHONY: build install-dev test

build:
	go build -o gh-review ./cmd/gh-review

install-dev: build
	mkdir -p "$(GH_EXTENSIONS_DIR)"
	ln -sfn "$(CURDIR)" "$(GH_EXTENSIONS_DIR)/$(GH_EXTENSION_NAME)"

test:
	go test ./...
