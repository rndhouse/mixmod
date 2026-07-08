CARGO ?= cargo
PYTHON ?= python3

.PHONY: check fmt build clippy test python-compile shell-compile status

check: fmt build clippy test python-compile shell-compile status

fmt:
	$(CARGO) fmt --check

build:
	$(CARGO) build

clippy:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test:
	$(CARGO) test

python-compile:
	$(PYTHON) -m compileall -q scripts

shell-compile:
	@for script in scripts/*.sh; do bash -n "$$script"; done

status: build
	MIXMOD_DEBUG_COMMANDS=1 target/debug/mixmod status >/dev/null
