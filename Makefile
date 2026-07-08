CARGO ?= cargo
PYTHON ?= python3

.PHONY: check fmt build clippy test python-compile status

check: fmt build clippy test python-compile status

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

status: build
	MIXMOD_DEBUG_COMMANDS=1 target/debug/mixmod status >/dev/null
