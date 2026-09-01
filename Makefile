SHELL := /bin/sh

CARGO ?= cargo
PUBLISH_POLL_SECONDS ?= 10
PUBLISH_MAX_ATTEMPTS ?= 60
PUBLISH_CRATES := \
	quickjs-jit-sys \
	quickjs-jit-core \
	quickjs-jit-macro \
	quickjs-jit \
	quickjs-jit-runtime

.PHONY: publish
publish:
	@set -eu; \
	for crate in $(PUBLISH_CRATES); do \
		version=$$($(CARGO) pkgid -p "$$crate" | sed 's/.*@//'); \
		package="$$crate@$$version"; \
		if $(CARGO) info --registry crates-io "$$package" >/dev/null 2>&1; then \
			echo "==> $$package is already published; skipping"; \
		else \
			echo "==> Publishing $$package"; \
			$(CARGO) publish -p "$$crate"; \
		fi; \
		attempt=0; \
		until $(CARGO) info --registry crates-io "$$package" >/dev/null 2>&1; do \
			attempt=$$((attempt + 1)); \
			if [ "$$attempt" -ge "$(PUBLISH_MAX_ATTEMPTS)" ]; then \
				echo "error: $$package is not visible on crates.io after $$attempt attempts" >&2; \
				exit 1; \
			fi; \
			echo "==> Waiting for $$package to appear on crates.io ($$attempt/$(PUBLISH_MAX_ATTEMPTS))"; \
			sleep "$(PUBLISH_POLL_SECONDS)"; \
		done; \
	done
