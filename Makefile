GH_EXTENSIONS_DIR ?= $(HOME)/.local/share/gh/extensions
GH_EXTENSION_NAME ?= gh-review
CURRENT_EXTENSION_NAME := $(notdir $(CURDIR))
GH_EXTENSION_PATH := $(GH_EXTENSIONS_DIR)/$(GH_EXTENSION_NAME)

.PHONY: build install reinstall install-dev test

build:
	go build -o gh-review ./cmd/gh-review

install: build
	mkdir -p "$(GH_EXTENSIONS_DIR)"
	@if [ -e "$(GH_EXTENSION_PATH)" ] && [ ! -L "$(GH_EXTENSION_PATH)" ]; then \
		echo "$(GH_EXTENSION_PATH) already exists and is not a symlink; run make reinstall"; \
		exit 1; \
	fi
	ln -sfn "$(CURDIR)" "$(GH_EXTENSION_PATH)"

reinstall:
	@gh extension remove "$(GH_EXTENSION_NAME)" >/dev/null 2>&1 || true
	@if [ "$(CURRENT_EXTENSION_NAME)" != "$(GH_EXTENSION_NAME)" ]; then \
		gh extension remove "$(CURRENT_EXTENSION_NAME)" >/dev/null 2>&1 || true; \
	fi
	$(MAKE) install

install-dev: install

test:
	go test ./...
