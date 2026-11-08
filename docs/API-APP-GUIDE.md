# LawGit アプリ開発者向け API 仕様書

この文書は、LawGit を利用する外部アプリ（GUI、Web UI、エディタ、同期ツールなど）の開発者向けガイドです。
API の機械可読な定義は [openapi.json](openapi.json)、詳細なメソッド一覧は [API.md](API.md) を参照してください。

- API バージョン: `0.2.0`
- HTTP ベース URL（既定）: `http://127.0.0.1:8787/api/v1`
- データ形式: UTF-8 JSON
- 認証: サーバー起動時に token を設定した場合に必要
- 外部クレート依存: LawGit 本体は標準ライブラリのみ

## 1. API の利用方法

アプリの構成に応じて、次のいずれかを選択してください。

| 接続方式 | 推奨用途 |
|---|---|
| HTTP API | GUI、Web アプリ、別プロセス・別言語から利用 |
| stdio RPC | LawGit を子プロセスとして起動するデスクトップアプリ、エディタ |
| Python クライアント | Python アプリ |
| JavaScript クライアント | Node.js / JavaScript アプリ |
| CLI の JSON 出力 | シェル、CI、簡単な連携 |

HTTP API は LawGit のプロセスが起動している間利用できます。対象リポジトリはサーバー起動時に指定します。

## 2. HTTP サーバー起動

ローカルアプリ向け（127.0.0.1 のみ）:

```sh
lawgit --repo /path/to/legal-data serve --host 127.0.0.1 --port 8787
```

トークン付きで LAN 等に公開する場合:

```sh
lawgit --repo /path/to/legal-data serve --host 0.0.0.0 --port 8787 --token YOUR_SECRET
```

トークンは環境変数 `LAWGIT_TOKEN` でも指定できます。非ループバックアドレスで起動する場合、トークンが必須です。

自動コミットを有効にする場合:

```sh
lawgit --repo /path/to/legal-data serve --commit
```

## 3. URL と HTTP メソッド

Base URL:

```text
http://127.0.0.1:8787/api/v1
```

| HTTP | パス | 用途 |
|---|---|---|
| GET | `/ping` | サーバー情報 |
| GET | `/methods` | RPC メソッド一覧 |
| GET | `/validate` | 法令・ルールの検証 |
| GET | `/openapi.json` | OpenAPI 3.0.3 定義 |
| GET | `/search?q=...&limit=...` | 全文検索 |
| GET | `/changes?since=...&limit=...` | commit の変更一覧・同期 |
| GET / POST | `/laws` | 法令一覧 / 作成 |
| GET | `/laws/{uuid}` | 法令と XML の取得 |
| GET | `/laws/{uuid}/history` | 法令の commit 履歴 |
| GET | `/laws/{uuid}/revisions/{rev}` | 指定 commit 時点の法令 XML |
| GET / POST | `/rules` | ルール一覧 / 作成 |
| GET / PUT / PATCH / DELETE | `/rules/{uuid}` | ルール取得 / 更新 / 削除 |
| GET | `/rules/{uuid}/history` | ルールの commit 履歴 |
| GET | `/rules/{uuid}/revisions/{rev}` | 指定 commit 時点のルール |
| GET | `/git/status` | Git 状態 |
| GET | `/git/log?limit=...` | commit 履歴 |
| POST | `/git/commit` | 検証付き commit |
| POST | `/rpc` | RPC メソッド呼び出し |

各リクエストの正確なパラメータ・スキーマは OpenAPI 定義を参照してください。

## 4. 共通のリクエスト・レスポンス形式

### 成功レスポンス

通常の API レスポンスは次の形式です。

```json
{
  "ok": true,
  "result": {}
}
```

`result` の内容は呼び出した API によって異なります。`/openapi.json` は例外で、OpenAPI ドキュメントを直接返します。

### エラーレスポンス

```json
{
  "ok": false,
  "error": "エラーの説明"
}
```

主な HTTP ステータス:

| Status | 意味 |
|---|---|
| 400 | 入力・パラメータ不正 |
| 401 | 認証トークン不正または不足 |
| 403 | Host / Origin 制限 |
| 404 | 対象が存在しない、またはルートが見つからない |
| 409 | バージョン競合 |
| 415 | Content-Type 不正 |
| 500 | 内部エラー |

書き込みリクエストは `Content-Type: application/json` を指定してください。

## 5. 認証

サーバー起動時に `--token` または `LAWGIT_TOKEN` を設定した場合、リクエストにトークンを付けます。

推奨ヘッダー:

```http
Authorization: Bearer YOUR_SECRET
```

互換ヘッダー:

```http
X-Lawgit-Token: YOUR_SECRET
```

トークンをブラウザの JavaScript ソース、公開リポジトリ、ログに埋め込まないでください。

## 6. 法令 API

### 法令一覧

```http
GET /api/v1/laws
```

一覧結果には法令の UUID、名前、種別、法令番号、e-Gov ID、パス、version 等が含まれます。

### 法令の取得

```http
GET /api/v1/laws/{uuid}
```

法令情報と XML テキストを取得します。UUID は LawGit が採番する UUID v4 を使用します。

### 法令の新規作成

```http
POST /api/v1/laws
Content-Type: application/json
```

RPC メソッド `law.create` の主なパラメータ:

```json
{
  "name": "新しい法令",
  "type": "法律",
  "law_number": "令和○年法律第○号",
  "e_gov_id": "optional-egov-id",
  "commit_message": "optional commit message"
}
```

`law_number`、`e_gov_id`、`commit_message` は任意です。

### 法令の履歴・過去版

- `GET /laws/{uuid}/history`: 対象法令に関連する commit 履歴
- `GET /laws/{uuid}/revisions/{rev}`: 指定 commit 時点の XML

削除済み法令の履歴も取得できます。

## 7. ルール API

### ルール一覧

```http
GET /api/v1/rules?category=人事&query=休暇
```

`category` と `query` は任意です。

### ルール作成

```http
POST /api/v1/rules
Content-Type: application/json
```

```json
{
  "name": "休暇規則",
  "category": "人事",
  "body": "# 休暇規則\n- 年次休暇について",
  "commit_message": "ルールを追加"
}
```

### ルール取得

```http
GET /api/v1/rules/{uuid}
```

レスポンスのルール情報には `body` と `version` が含まれます。

### ルール更新

```http
PUT /api/v1/rules/{uuid}
Content-Type: application/json
```

```json
{
  "body": "# 更新された休暇規則",
  "if_version": "取得時に返された version",
  "commit_message": "休暇規則を更新"
}
```

更新可能な項目は `name`、`category`、`body` です。PATCH も利用できます。更新時には競合防止のため `if_version` を指定してください。

### ルール削除

```http
DELETE /api/v1/rules/{uuid}?if_version=VERSION
```

削除時も、取得時の `version` を `if_version` として渡すことを推奨します。必要に応じて `commit_message` を指定できます。

### ルールの履歴・過去版

- `GET /rules/{uuid}/history`
- `GET /rules/{uuid}/revisions/{rev}`

削除済みルールについても履歴・過去版を取得できます。

## 8. 同時編集と version

複数アプリが同じ法令・ルールを編集する可能性がある場合、取得結果の `version` を保存してください。

更新・削除時に `if_version` を送信すると、取得後に他のアプリが更新していた場合は HTTP 409（Conflict）になります。競合時は最新データを再取得し、ユーザーの編集内容を再適用してください。

**取得時の version を省略した更新は、同時編集に対して安全とは限りません。**

## 9. 変更の同期・監視

```http
GET /api/v1/changes?limit=1
```

返された `head`（現在の先頭 commit）をクライアント側に保存します。

次回以降:

```http
GET /api/v1/changes?since={保存したhead}
```

`since` 以降の commit と変更ファイルが返されます。結果は新しい commit 順です。クライアントは取得した commit を必要に応じて古い順に処理してください。

同期処理では、commit hash をチェックポイントとして保存し、再起動後に同じ位置から再開できるようにします。

## 10. RPC

stdio RPC と HTTP RPC は共通のメソッド群を使用します。

HTTP RPC:

```http
POST /api/v1/rpc
Content-Type: application/json
```

```json
{
  "method": "rule.create",
  "params": {
    "name": "休暇規則",
    "category": "人事",
    "body": "# 休暇規則"
  }
}
```

stdio RPC は LawGit を子プロセスとして起動します。

```sh
lawgit --repo legal-data rpc
```

標準入力・標準出力は JSON Lines（1 行につき 1 JSON リクエスト / レスポンス）です。stdio RPC のリクエストには `id` を付け、応答の `id` と対応付けてください。

主な RPC メソッド:

- `ping`, `methods`
- `law.list`, `law.get`, `law.create`, `law.validate`, `law.history`, `law.revision`
- `rule.list`, `rule.get`, `rule.create`, `rule.update`, `rule.delete`, `rule.validate`, `rule.history`, `rule.revision`
- `search`, `changes`, `repo.check`
- `git.status`, `git.log`, `git.commit`

## 11. 自動コミット

自動コミットはサーバー起動時の `--commit`、`LAWGIT_AUTOCOMMIT`、またはリポジトリの `git config lawgit.autocommit` で制御します。

優先順位:

1. `--commit` / `--no-commit`
2. `LAWGIT_AUTOCOMMIT`
3. `git config lawgit.autocommit`
4. 既定 OFF

API の書き込みに `commit_message` を指定すると、既定の commit メッセージを上書きできます。

commit に失敗した場合、ファイル変更自体は保存済みであり、API の結果に commit エラーが含まれます。アプリは「変更保存」と「Git commit 成功」を別々に表示してください。

## 12. ブラウザアプリ（CORS）

既定では CORS を許可しません。ブラウザからアクセスする場合は、サーバー起動時に許可する Origin を完全一致で指定します。

```sh
lawgit serve --token YOUR_SECRET --cors-origin https://app.example.com
```

- Origin は完全一致で指定します。
- ワイルドカード `*` は使用できません。
- `--cors-origin` を使う場合は `--token` が必須です。
- プリフライト `OPTIONS` は許可 Origin に対して 204 を返します。
- トークンはブラウザに埋め込むと利用者に露出するため、公開 Web アプリではバックエンド経由の接続を検討してください。

## 13. セキュリティ・運用上の制約

- 既定の bind address は `127.0.0.1` です。
- 非ループバックでの起動には token が必要です。
- TLS は LawGit 自体では提供しません。外部通信では TLS 対応リバースプロキシ等を使用してください。
- 1 接続ずつ逐次処理します。
- リクエストサイズ上限は 1 MiB、タイムアウトは 10 秒です。
- 書き込み系では JSON Content-Type が必要です。
- UUID と commit ID は形式検証されます。パス文字列を任意に指定する API として扱わないでください。
- 法令・ルールの XML や Markdown 本文は、アプリ側でも HTML 表示時に適切にエスケープしてください。

## 14. クライアントライブラリ

### Python

`clients/lawgit_client.py`:

```python
from lawgit_client import LawGitClient

client = LawGitClient("http://127.0.0.1:8787")
```

### JavaScript / Node.js

`clients/lawgit_client.mjs` を使用します。Node.js 18 以降を対象とし、外部依存はありません。

具体的な関数・引数は各クライアントのソースとテストを参照してください。

## 15. 仕様の参照先

- [OpenAPI 3.0.3](openapi.json): HTTP API の機械可読仕様
- [API.md](API.md): API の概要・メソッド表・接続例
- [README](../README.md): CLI とリポジトリ運用


---

## 実装仕様の補足（API 共通メソッド群）

以下は `src/api.rs` の共通メソッド群に基づく補足です。CLI JSON、stdio JSON-lines、HTTP は同じ API メソッドを共有します。

### RPC メソッド一覧

| method | params | result |
|---|---|---|
| `ping` | なし | `{name, version, root, auto_commit}` |
| `methods` | なし | メソッド名配列 |
| `law.list` | なし | `{laws:[{uuid,name,type,law_number,e_gov_id,path,version}], errors:[]}` |
| `law.get` | `uuid` | `{law, xml}` |
| `law.create` | `name, type, law_number?, e_gov_id?, commit_message?` | 作成した法令（＋commit） |
| `law.validate` | なし | `{valid, laws, errors}` |
| `law.history` | `uuid, limit?` | `{commits:[{hash,author,date,subject}]}` |
| `law.revision` | `uuid, rev` | `{rev, xml}` |
| `rule.list` | `category?, query?` | `{rules:[...], errors:[]}` |
| `rule.get` | `uuid` | ルール（body・version を含む） |
| `rule.create` | `name, category?, body?, commit_message?` | 作成したルール（＋commit） |
| `rule.update` | `uuid, name?, category?, body?, if_version?, commit_message?` | 更新後ルール（＋commit） |
| `rule.delete` | `uuid, if_version?, commit_message?` | `{deleted}`（＋commit） |
| `rule.validate` | なし | `{valid, rules, errors}` |
| `rule.history` / `rule.revision` | `law.*` の履歴系と同様 | 削除済みルールの履歴・過去版も取得可能 |
| `search` | `q, limit?`（既定 50） | `{query, hits:[{kind,uuid,name,matched,snippet}], truncated}` |
| `changes` | `since?, limit?` | `{head, commits:[{hash,date,subject,files:[{status,path}]}]}`（新しい順） |
| `repo.check` | なし | `{valid, laws, rules, errors}` |
| `git.status` | なし | Git 状態 |
| `git.log` | `limit?` | Git 履歴 |
| `git.commit` | `message` | 検証付き一括 commit |

### 自動コミットの詳細

LawGit 経由の変更を自動 commit する設定:

```sh
lawgit config autocommit on
lawgit config autocommit off
lawgit rule new --name X --commit
LAWGIT_AUTOCOMMIT=1 lawgit serve
lawgit serve --commit
```

優先順位:

1. `--commit` / `--no-commit`
2. `LAWGIT_AUTOCOMMIT`
3. `git config lawgit.autocommit`
4. OFF

変更したファイルだけを対象に 1 変更 = 1 commit とし、他のステージ済み作業を巻き込みません。内容に変化がなければ空 commit は作成しません。CLI は `-m)、API は `commit_message` でメッセージを指定できます。

commit に失敗してもファイル変更は残ります。CLI は終了コード 1、API は結果の `commit` に `{"error":"..."}` を返します。アプリは保存成功と commit 成功を分けて表示してください。自動コミット状態は `ping` の `auto_commit` で確認できます。

### 競合検出

取得結果の `version` はファイル内容のハッシュです。更新・削除時に `if_version` を送ると、取得後に他のクライアントが変更した場合、HTTP 409 / conflict となり書き込みは行われません。

推奨手順:

1. GET でデータと `version` を取得。
2. 編集中は version を保持。
3. PUT / PATCH / DELETE に `if_version` を付ける。
4. 409 の場合は最新データを再取得し、差分をユーザーに提示。

Python クライアントの `edit_rule(uuid, callback)` は読み取り→更新→競合時の再試行を行います。callback は最新データに対して安全に再適用できる処理にしてください。

### changes / watch による同期

`changes` API は push 通知の代わりにポーリングで更新を追従する仕組みです。

初回に `GET /api/v1/changes?limit=1` を呼び、返却された `head` を保存します。その後は `GET /api/v1/changes?since=<保存した head>` を呼び、差分 commit と変更ファイルを取得します。結果は新しい順なので、順序依存の処理では古い順に並べ替えてください。

Python クライアント:

```python
for commit in c.watch():  # デフォルト 2 秒間隔、古い commit から yield
    print(commit["subject"], [f["path"] for f in commit["files"]])
```

commit hash を同期 checkpoint として保存し、再起動後に再開できるようにしてください。

### stdio JSON-lines

```sh
printf '%s\\n' '{"id":1,"method":"ping"}' \
  '{"id":2,"method":"rule.create","params":{"name":"休暇規則","category":"人事","body":"# 休暇"}}' \
  | lawgit --repo legal-data rpc
```

1 行につき 1 リクエスト。レスポンスはリクエストの `id` を返す 1 行 JSON です。

### HTTP API パス一覧

| HTTP | Path |
|---|---|
| GET | `/api/v1/ping`, `/methods`, `/validate`, `/openapi.json` |
| GET | `/api/v1/search?q=...&limit=...` |
| GET | `/api/v1/changes?since=...&limit=...` |
| GET / POST | `/api/v1/laws` |
| GET | `/api/v1/laws/{uuid}`, `/laws/{uuid}/history`, `/laws/{uuid}/revisions/{rev}` |
| GET / POST | `/api/v1/rules`（GET: `?category=&query=`） |
| GET / PUT / PATCH / DELETE | `/api/v1/rules/{uuid}` |
| GET | `/api/v1/rules/{uuid}/history`, `/rules/{uuid}/revisions/{rev}` |
| GET | `/api/v1/git/status`, `/git/log?limit=...` |
| POST | `/api/v1/git/commit`, `/api/v1/rpc` |

### HTTP 呼び出し例

ルール作成:

```sh
curl -s -X POST http://localhost:8787/api/v1/rules \
  -H 'Content-Type: application/json' \
  -d '{"name":"休暇規則","category":"人事","body":"# 休暇"}'
```

ルール更新（取得した version を使う）:

```sh
curl -s -X PUT http://localhost:8787/api/v1/rules/UUID \
  -H 'Content-Type: application/json' \
  -d '{"body":"# 更新","if_version":"取得時のversion"}'
```

token 認証:

```sh
curl -s -H 'Authorization: Bearer SECRET' http://host:8787/api/v1/ping
```

### CORS

- 既定では CORS を許可しません。
- `--cors-origin` に許可 Origin を完全一致で指定します。複数はカンマ区切り。
- ワイルドカード `*` は不可。CORS 設定時は `--token` が必須。
- OPTIONS プリフライトは許可 Origin に 204 を返し、token は実リクエストで検証します。

### 安全設計と制限

- 既定は `127.0.0.1`。非ループバック bind は token 必須。
- ループバックで token 無しの場合、Host ヘッダー検証により DNS rebinding を防止。
- 書き込みは JSON Content-Type 必須。
- UUID / commit ID は形式検証後に利用し、パストラバーサル・Git オプション注入を防ぐ。
- 1 接続ずつ逐次処理。
- リクエスト上限 1 MiB、タイムアウト 10 秒。
- TLS は LawGit 本体では提供しない。外部公開時は TLS リバースプロキシを使用する。
