# LawGit 連携 API / Integration API

他アプリとの接続口は 3 つあり、すべて同じメソッド群（`src/api.rs`）を共有します。

| 窓口 | 起動 | 用途 |
|---|---|---|
| CLI `--json` | `lawgit list --json` など | シェルスクリプト・CI |
| stdio JSON-lines | `lawgit --repo DIR rpc` | 子プロセスとして埋め込む（エディタ、GUI、他言語） |
| ローカル HTTP | `lawgit --repo DIR serve` | 常駐サービス、Web UI、他サービス |

## メソッド一覧

| method | params | result |
|---|---|---|
| `ping` | – | `{name, version, root}` |
| `methods` | – | メソッド名の配列 |
| `law.list` | – | `{laws:[{uuid,name,type,law_number,e_gov_id,path}], errors:[]}` |
| `law.get` | `uuid` | `{law:{...}, xml:"..."}` |
| `law.create` | `name`, `type`, `law_number?`, `e_gov_id?` | 作成した法令 |
| `law.validate` | – | `{valid, laws, errors}` |
| `rule.list` | `category?`, `query?` | `{rules:[{uuid,name,category,created,path}], errors:[]}` |
| `rule.get` | `uuid` | ルール（`body` を含む） |
| `rule.create` | `name`, `category?`, `body?` | 作成したルール |
| `rule.update` | `uuid`, `name?`, `category?`, `body?` | 更新後のルール |
| `rule.delete` | `uuid` | `{deleted}` |
| `rule.validate` | – | `{valid, rules, errors}` |
| `repo.check` | – | `{valid, laws, rules, errors}` |
| `git.status` | – | `{clean, output}` |
| `git.log` | `limit?`(既定20) | `{commits:[{hash,author,date,subject}]}` |
| `git.commit` | `message` | `{output}`（検証に通った場合のみ laws/・rules/ を add して commit） |

レスポンスは常に `{"ok":true,"result":...}` または `{"ok":false,"error":"..."}`。

## stdio（JSON-lines）

```sh
printf '%s\n' '{"id":1,"method":"ping"}' \
              '{"id":2,"method":"rule.create","params":{"name":"休暇規則","category":"人事","body":"# 休暇\n- ..."}}' \
  | lawgit --repo legal-data rpc
```

各行 1 リクエスト。応答は `id` をそのまま返す 1 行 JSON。

## HTTP

```sh
lawgit --repo legal-data serve --port 8787            # 127.0.0.1 のみ
lawgit --repo legal-data serve --host 0.0.0.0 --token SECRET   # 外部公開はトークン必須
```

| メソッド | パス |
|---|---|
| GET | `/api/v1/ping`, `/api/v1/methods`, `/api/v1/validate` |
| GET / POST | `/api/v1/laws` |
| GET | `/api/v1/laws/{uuid}` |
| GET / POST | `/api/v1/rules`（GET は `?category=&query=`） |
| GET / PUT・PATCH / DELETE | `/api/v1/rules/{uuid}` |
| GET | `/api/v1/git/status`, `/api/v1/git/log?limit=20` |
| POST | `/api/v1/git/commit` |
| POST | `/api/v1/rpc`（`{"method":...,"params":...}`） |

```sh
curl -s localhost:8787/api/v1/rules
curl -s -X POST localhost:8787/api/v1/rules -H 'Content-Type: application/json' \
     -d '{"name":"休暇規則","category":"人事","body":"# 休暇"}'
curl -s -X PATCH localhost:8787/api/v1/rules/<UUID> -H 'Content-Type: application/json' \
     -d '{"body":"# 更新"}'
curl -s -H 'Authorization: Bearer SECRET' http://host:8787/api/v1/ping   # --token 使用時
```

### 安全設計
- 既定は 127.0.0.1 のみ。非ループバックは `--token`（または環境変数 `LAWGIT_TOKEN`）が無いと起動しない。
- ループバックでトークン無しの場合は Host ヘッダを検証（DNS rebinding 対策）。
- 書き込み系は `Content-Type: application/json` 必須。CORS ヘッダは返さないため、ブラウザの別オリジンからは書き込めない。
- UUID は厳密な形式検証を通ったものだけがパスになる（パストラバーサル不可）。
- 1 接続ずつ逐次処理（ファイル競合を避ける）。リクエスト 1 MiB 上限、10 秒タイムアウト。TLS は提供しないため、外部公開時はリバースプロキシで TLS を終端すること。
