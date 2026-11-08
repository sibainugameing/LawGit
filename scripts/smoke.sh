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

# auto-commit, search, history
"$BIN" --repo "$D" config autocommit on
U=$("$BIN" --repo "$D" rule new --name "自動コミット規則" --body "年次有給休暇" | sed -n 's/^UUID: //p')
"$BIN" --repo "$D" rule set "$U" --body "年次有給休暇は年5日"
"$BIN" --repo "$D" search 有給
"$BIN" --repo "$D" history "$U"
git -C "$D" log --oneline
"$BIN" --repo "$D" openapi | head -c 80; echo

# HTTP
"$BIN" --repo "$D" serve --port 18787 &
PID=$!
sleep 1
curl -sf http://127.0.0.1:18787/api/v1/ping
curl -sf http://127.0.0.1:18787/api/v1/rules
curl -sf -X POST http://127.0.0.1:18787/api/v1/rules \
  -H 'Content-Type: application/json' -d '{"name":"API経由","body":"x"}'
curl -sf "http://127.0.0.1:18787/api/v1/search?q=%E6%9C%89%E7%B5%A6"
curl -sf "http://127.0.0.1:18787/api/v1/changes?limit=3"
curl -sf http://127.0.0.1:18787/api/v1/openapi.json | head -c 80
kill $PID
wait $PID 2>/dev/null || true
echo
echo "SMOKE TEST PASSED"
