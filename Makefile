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
		version=$$($(CARGO) pkgid -p "$$crate" | sed 's/.*[@#]//'); \
		package="$$crate@$$version"; \
		if $(CARGO) info "$$package" >/dev/null 2>&1; then \
			echo "=> $$package is already published; skipping"; \
		else \
			echo "=> Publishing $$package"; \
			$(CARGO) publish -p "$$crate"; \
		fi; \
	done
