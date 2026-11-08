# LawGit 連携 API / Integration API

他アプリとの接続口は 4 つあり、すべて同じメソッド群（`src/api.rs`）を共有します。

| 窓口 | 起動 | 用途 |
|---|---|---|
| CLI `--json` | `lawgit list --json` など | シェルスクリプト・CI |
| stdio JSON-lines | `lawgit --repo DIR rpc` | 子プロセスとして埋め込む（エディタ、GUI、他言語） |
| ローカル HTTP | `lawgit --repo DIR serve` | 常駐サービス、Web UI、他サービス |
| クライアント | `clients/lawgit_client.py` / `.mjs` | HTTP API を 1 行で呼べる依存なしライブラリ |

機械可読な仕様は **OpenAPI 3**: `GET /api/v1/openapi.json`（`lawgit openapi` でも出力、`docs/openapi.json` に同梱）。
コード生成ツール（openapi-generator など）にそのまま渡せます。

レスポンスは常に `{"ok":true,"result":...}` または `{"ok":false,"error":"..."}`（`openapi.json` 自体を除く）。

## メソッド一覧

| method | params | result |
|---|---|---|
| `ping` | – | `{name, version, root, auto_commit}` |
| `methods` | – | メソッド名の配列 |
| `law.list` | – | `{laws:[{uuid,name,type,law_number,e_gov_id,path,version}], errors:[]}` |
| `law.get` | `uuid` | `{law:{...}, xml:"..."}` |
| `law.create` | `name`, `type`, `law_number?`, `e_gov_id?`, `commit_message?` | 作成した法令（＋`commit`） |
| `law.validate` | – | `{valid, laws, errors}` |
| `law.history` | `uuid`, `limit?` | `{commits:[{hash,author,date,subject}]}`（削除済みでも可） |
| `law.revision` | `uuid`, `rev` | `{rev, xml}` その commit 時点の XML |
| `rule.list` | `category?`, `query?` | `{rules:[...], errors:[]}` |
| `rule.get` | `uuid` | ルール（`body`・`version` を含む） |
| `rule.create` | `name`, `category?`, `body?`, `commit_message?` | 作成したルール（＋`commit`） |
| `rule.update` | `uuid`, `name?`, `category?`, `body?`, `if_version?`, `commit_message?` | 更新後のルール（＋`commit`） |
| `rule.delete` | `uuid`, `if_version?`, `commit_message?` | `{deleted}`（＋`commit`） |
| `rule.validate` | – | `{valid, rules, errors}` |
| `rule.history` / `rule.revision` | `law.*` と同じ | 削除済みルールの履歴・過去版も取得でき、復元に使える |
| `search` | `q`, `limit?`(既定50) | `{query, hits:[{kind,uuid,name,matched,snippet}], truncated}` |
| `changes` | `since?`, `limit?` | `{head, commits:[{hash,date,subject,files:[{status,path}]}]}`（新しい順） |
| `repo.check` | – | `{valid, laws, rules, errors}` |
| `git.status` / `git.log` / `git.commit` | – / `limit?` / `message` | Git の状態・履歴・検証付き一括 commit |

## 自動コミット

有効にすると、LawGit 経由の変更（`new`、`rule new/make/set/edit/delete`、API の書き込み）が
**「変更したファイルだけ」を 1 変更 = 1 commit** で記録します（他にステージ済みの作業は巻き込みません。内容が変わらなければ空 commit は作りません）。

```sh
lawgit config autocommit on        # このリポジトリの git config に保存（commit されない・共有されない）
lawgit config autocommit off
lawgit rule new --name X --commit  # この実行だけ ON  (--no-commit で OFF)
LAWGIT_AUTOCOMMIT=1 lawgit serve   # 環境変数でも指定可
lawgit serve --commit              # サーバの書き込みだけ自動コミット
```

優先順位: `--commit/--no-commit` > 環境変数 `LAWGIT_AUTOCOMMIT` > `git config lawgit.autocommit` > OFF。
メッセージの既定は「法令を追加: 名前 [UUID]」「ルールを追加/更新/編集/削除: 名前 [UUID]」。CLI は `-m`、API は `commit_message` で上書きできます。
commit に失敗しても保存済みの変更は残り、CLI は終了コード 1、API は結果の `commit` に `{"error":...}` を返します
（`user.name` / `user.email` 未設定が典型的な原因）。有効時は `ping` の `auto_commit` が `true` になります。

## 複数アプリが同時に編集するとき（競合検出）

`law.*` / `rule.*` の取得結果には `version`（ファイル内容のハッシュ）が含まれます。
更新・削除時に `if_version` を渡すと、その間に他者が変更していた場合は **HTTP 409 / `conflict: ...`** になり、何も書き込まれません。

```python
c.edit_rule(uuid, lambda r: {"body": r["body"] + "\n- 追記"})   # 読み取り→更新→競合なら自動リトライ
```

## 変更の追従（同期・通知）

push 通知の代わりに、軽量なポーリング用 API を用意しています。

```sh
curl 'localhost:8787/api/v1/changes?limit=1'                 # → head（現在の先頭 commit）を保存
curl 'localhost:8787/api/v1/changes?since=<保存した head>'    # → それ以降の commit と変更ファイルだけ
```

```python
for commit in c.watch():      # 起動時点以降の commit を古い順に yield（既定 2 秒間隔）
    print(commit["subject"], [f["path"] for f in commit["files"]])
```

## stdio（JSON-lines）

```sh
printf '%s\n' '{"id":1,"method":"ping"}' \
              '{"id":2,"method":"rule.create","params":{"name":"休暇規則","category":"人事","body":"# 休暇\n- ..."}}' \
  | lawgit --repo legal-data rpc
```

各行 1 リクエスト。応答は `id` をそのまま返す 1 行 JSON。

## HTTP

```sh
lawgit --repo legal-data serve --port 8787                       # 127.0.0.1 のみ
lawgit --repo legal-data serve --host 0.0.0.0 --token SECRET     # 外部公開はトークン必須
lawgit --repo legal-data serve --token SECRET --cors-origin https://app.example.com   # ブラウザアプリ用
```

| メソッド | パス |
|---|---|
| GET | `/api/v1/ping`, `/methods`, `/validate`, `/openapi.json` |
| GET | `/api/v1/search?q=...&limit=` |
| GET | `/api/v1/changes?since=&limit=` |
| GET / POST | `/api/v1/laws` |
| GET | `/api/v1/laws/{uuid}`, `/laws/{uuid}/history`, `/laws/{uuid}/revisions/{rev}` |
| GET / POST | `/api/v1/rules`（GET は `?category=&query=`） |
| GET / PUT・PATCH / DELETE | `/api/v1/rules/{uuid}`（DELETE は `?if_version=&commit_message=`） |
| GET | `/api/v1/rules/{uuid}/history`, `/rules/{uuid}/revisions/{rev}` |
| GET | `/api/v1/git/status`, `/git/log?limit=` |
| POST | `/api/v1/git/commit`, `/api/v1/rpc`（`{"method":...,"params":...}`） |

```sh
curl -s -X POST localhost:8787/api/v1/rules -H 'Content-Type: application/json' \
     -d '{"name":"休暇規則","category":"人事","body":"# 休暇"}'
curl -s -X PUT localhost:8787/api/v1/rules/<UUID> -H 'Content-Type: application/json' \
     -d '{"body":"# 更新","if_version":"<取得時の version>"}'
curl -s -H 'Authorization: Bearer SECRET' http://host:8787/api/v1/ping        # --token 使用時
```

ステータス: 400 不正な入力 / 401 トークン / 403 Host・Origin / 404 見つからない / 409 競合 / 415 Content-Type / 500 内部エラー。

### ブラウザ（CORS）
- 既定では CORS ヘッダを返しません（他サイトのページからは読み書きできません）。
- `--cors-origin` に**完全一致**のオリジン（カンマ区切りで複数可）を指定したときだけ、そのオリジンに限り許可します。ワイルドカードは不可、**`--token` が必須**です。
- プリフライト（`OPTIONS`）は許可オリジンにのみ 204 を返し、トークン検証はその後の実リクエストで行います。

### 安全設計
- 既定は 127.0.0.1 のみ。非ループバックは `--token`（または環境変数 `LAWGIT_TOKEN`）が無いと起動しない。
- ループバックでトークン無しの場合は Host ヘッダを検証（DNS rebinding 対策）。
- 書き込み系は `Content-Type: application/json` 必須。
- UUID・commit id は厳密な形式検証を通ったものだけがパス／git 引数になる（パストラバーサル・オプション注入不可）。
- 1 接続ずつ逐次処理（ファイル競合を避ける）。リクエスト 1 MiB 上限、10 秒タイムアウト。TLS は提供しないため、外部公開時はリバースプロキシで TLS を終端すること。
