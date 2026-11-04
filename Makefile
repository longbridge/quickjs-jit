SHELL := /bin/sh

CARGO ?= cargo
PUBLISH_POLL_SECONDS ?= 10
PUBLISH_MAX_ATTEMPTS ?= 60
CRATES_INDEX_URL ?= https://index.crates.io
PUBLISH_CRATES := \
	quickjs-jit-sys \
	quickjs-jit-core \
	quickjs-jit-macro \
	quickjs-jit \
	quickjs-jit-runtime \
	quickjs-jit-stdlib

.PHONY: publish-dry-run
publish-dry-run:
	$(CARGO) publish --dry-run $(foreach crate,$(PUBLISH_CRATES),-p $(crate))

.PHONY: publish
publish:
	@set -eu; \
	for crate in $(PUBLISH_CRATES); do \
		version=$$($(CARGO) pkgid -p "$$crate" | sed 's/.*[@#]//'); \
		if [ -z "$$version" ]; then \
			echo "error: cargo pkgid returned no version for $$crate; run 'cargo generate-lockfile' and retry" >&2; \
			exit 1; \
		fi; \
		package="$$crate@$$version"; \
		prefix=$$(printf '%s' "$$crate" | cut -c1-2)/$$(printf '%s' "$$crate" | cut -c3-4); \
		response=$$(curl -sSL --retry 3 -w '\n%{http_code}' "$(CRATES_INDEX_URL)/$$prefix/$$crate"); \
		status=$$(printf '%s\n' "$$response" | tail -n 1); \
		case "$$status" in \
			200) published=$$(printf '%s\n' "$$response" | grep -cF "\"vers\":\"$$version\"" || true) ;; \
			404|410) published=0 ;; \
			*) echo "error: crates.io index returned HTTP $$status for $$crate" >&2; exit 1 ;; \
		esac; \
		if [ "$$published" -gt 0 ]; then \
			echo "=> $$package is already published; skipping"; \
		else \
			echo "=> Publishing $$package"; \
			$(CARGO) publish -p "$$crate"; \
		fi; \
	done
