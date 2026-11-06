#!/bin/sh
# End-to-end smoke test. Run after `cargo build --release`.
set -eu
BIN="${BIN:-./target/release/lawgit}"
D="$(mktemp -d)"
trap 'rm -rf "$D"' EXIT

"$BIN" --repo "$D" init
"$BIN" --repo "$D" new --name "道路交通法" --type "法律" \
  --law-number "昭和三十五年法律第百五号" --e-gov-id "335AC0000000105"
"$BIN" --repo "$D" validate
"$BIN" --repo "$D" list
"$BIN" --repo "$D" rule new --name "休暇規則" --category "人事" --body "# 休暇"
printf '運用規則\n運用\n# 見出し\n.. はリテラル\n.\n' | "$BIN" --repo "$D" rule make
"$BIN" --repo "$D" rule list
"$BIN" --repo "$D" check
"$BIN" --repo "$D" rule list --json
printf '%s\n' '{"id":1,"method":"ping"}' '{"id":2,"method":"repo.check"}' | "$BIN" --repo "$D" rpc

# HTTP
"$BIN" --repo "$D" serve --port 18787 &
PID=$!
sleep 1
curl -sf http://127.0.0.1:18787/api/v1/ping
curl -sf http://127.0.0.1:18787/api/v1/rules
curl -sf -X POST http://127.0.0.1:18787/api/v1/rules \
  -H 'Content-Type: application/json' -d '{"name":"API経由","body":"x"}'
kill $PID
echo
echo "SMOKE TEST PASSED"
