#!/usr/bin/env sh
# BEGIN MIXMOD MANAGED: hook
set -eu

depth="${MIXMOD_HOOK_DEPTH:-0}"
if [ "$depth" -gt 0 ]; then
  exit 0
fi

export MIXMOD_HOOK_DEPTH=1
exec mixmod hook "$@"
# END MIXMOD MANAGED: hook
