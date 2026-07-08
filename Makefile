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
	$(PYTHON) -m py_compile \
		scripts/deepswe_codex_app_agent.py \
		scripts/deepswe_mixmod_agent.py \
		scripts/deepswe_run_mixmod_selected.py \
		scripts/deepswe_screen_codex_pass.py \
		scripts/swebench_run_mixmod_selected.py \
		scripts/swebench_screen_codex_pass.py

status: build
	MIXMOD_DEBUG_COMMANDS=1 target/debug/mixmod status >/dev/null
