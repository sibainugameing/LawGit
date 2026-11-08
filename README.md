# LawGit

法令とユーザー作成ルールを、構造化 XML として Git リポジトリで管理する Rust 製 CLI です。
LawGit is a Rust CLI that manages legal documents and user-authored rules as structured XML in a Git repository.

データ形式とリポジトリ構成は [standard-jp.md](standard-jp.md) / [standard-en.md](standard-en.md) に従います（ルール文書は付録 A）。
**外部クレート依存なし**（標準ライブラリのみ）なので、Rust と Git があればビルドできます。

## ビルド / Build

```sh
cargo build --release
cargo test                  # 単体テスト + tests/cli.rs（実バイナリで CLI・rpc・HTTP・commit を通しで検証）
sh scripts/smoke.sh         # CLI・RPC・HTTP の通し確認（curl 必要）
cargo install --path .      # lawgit と git-law を ~/.cargo/bin へ
```

`git-law` を PATH に置くと `git law ...` で呼べます（Git の外部サブコマンド機構。Git 本体は変更しません）。
ヘルプは `git law help`（`git law --help` は Git が man を探しに行くため）。

## 検証状況（正直な申告）

- **v0.1 の Rust 部分**: 利用者の環境で `cargo build --release && cargo test && sh scripts/smoke.sh` が全件通過したことを確認済みです（単体 16 件・統合 9 件・スモークテスト）。
- **v0.2 で追加した Rust コード**（自動コミット・検索・履歴・競合検出・CORS・OpenAPI と、その統合テスト）: 開発環境にコンパイラが無く、**まだコンパイル・実行していません**。机上レビューと括弧対応の機械検査のみです。
  同じコマンドで検証してください（エラーが出たら全文を共有してください）。`.github/workflows/ci.yml` を push すれば CI でも走ります。
- **Python / JavaScript 側**（`tools/`, `clients/`）: テスト済み（tools 23 件、Python クライアント 8 件、JS クライアント 3 件。クライアントはモックサーバー相手で、実サーバーとの結合は Rust の統合テストが担当）。
- **e-Gov API への実通信**は未確認です（`tools/egov_import.py` は dry-run・`--from-file` で安全に試せます）。

## クイックスタート

```sh
mkdir legal-data
git law --repo legal-data init

# 法令: UUID v4 を採番し laws/<UUID>.xml を作成
git law --repo legal-data new --name "道路交通法" --type "法律" \
  --law-number "昭和三十五年法律第百五号" --e-gov-id "335AC0000000105"
# → XML の <body> に条・項・号を XML 要素で記述（単一文字列は不可）
git law --repo legal-data validate
git law --repo legal-data list
git law --repo legal-data show <UUID>
```

## ルールの作成・管理

ルールは `rules/<UUID>.xml`（ルート `rule`）に保存され、法令とは別スキーマ・別検証です。UUID は不変です。

```sh
lawgit rule make                      # 対話: 名前 → カテゴリ → 本文(Markdown)。単独の . で終了、.. でリテラルの .
lawgit rule new --name "休暇規則" --category 人事 --body-file vacation.md   # 非対話
lawgit rule list [--category 人事] [--query 休暇]
lawgit rule show <UUID>
lawgit rule set <UUID> --name "新名称" --body-file new.md   # 名前/カテゴリ/本文を更新（UUID・作成日時は保持）
lawgit rule edit <UUID>               # $VISUAL / $EDITOR で編集 → 保存後に自動検証
lawgit rule path <UUID>
lawgit rule delete <UUID> --yes
lawgit rule validate
lawgit check                          # laws/ と rules/ をまとめて検証（UUID 重複も検出）
```

## Git ワークフロー

Git が履歴エンジンです。LawGit は独自の履歴ファイルを作りません。既定では自動 commit しません（下の「自動コミット」で有効化できます）。

```sh
lawgit --repo legal-data validate
cd legal-data
git add laws/<UUID>.xml
git commit -m "道路交通法：改正内容を記述"
```

1 改正 = 1 commit。1 つの改正法が複数の法令を変更する場合は、影響する全 XML を同じ commit に含め、改正法と変更概要を commit メッセージに記録します。
改正案は branch、採択は merge、名前を付けたい状態は tag を使います。

便利コマンド: `lawgit commit -m "メッセージ"`（全体を検証し、通った場合のみ `laws/` と `rules/` を add して commit）。

### 自動コミット

```sh
lawgit config autocommit on     # このリポジトリで常に ON（git config に保存。共有されない）
lawgit config autocommit off
lawgit rule new --name X --commit        # この実行だけ ON（--no-commit で OFF）  /  LAWGIT_AUTOCOMMIT=1 でも可
lawgit rule set <UUID> --body "..." -m "休暇日数を変更"   # メッセージ指定
lawgit serve --commit            # HTTP API 経由の書き込みも自動コミット
```

- ON の間、`new` / `rule new・make・set・edit・delete` と API の書き込みは、**変更したファイルだけ**を **1 変更 = 1 commit** で記録します。
  他にステージ済みの作業は巻き込まず、内容が変わらない場合は空 commit を作りません。
- 既定のメッセージは「ルールを更新: 休暇規則 [UUID]」のような形式です。
- commit に失敗（`user.name` / `user.email` 未設定など）しても保存済みの変更は残り、終了コード 1 と警告を返します。
- 変更の履歴は `lawgit history <UUID>`（削除済みも可）、全文検索は `lawgit search 語句` で確認できます。

## 他アプリとの連携窓口 / Integration

| 窓口 | コマンド |
|---|---|
| JSON 出力 | `lawgit list --json`, `show --json`, `validate --json`, `check --json`, `rule list --json` … |
| stdio JSON-lines | `lawgit --repo DIR rpc` |
| ローカル HTTP API | `lawgit --repo DIR serve [--host 127.0.0.1] [--port 8787] [--token T] [--cors-origin ORIGIN] [--commit]` → `/api/v1/...` |
| OpenAPI 3 仕様 | `GET /api/v1/openapi.json` / `lawgit openapi` / [docs/openapi.json](docs/openapi.json)（コード生成にそのまま使える） |
| クライアント | [clients/lawgit_client.py](clients/lawgit_client.py)（Python）・[clients/lawgit_client.mjs](clients/lawgit_client.mjs)（Node 18+/ブラウザ）。依存なし |

主な機能: 法令・ルールの CRUD、**全文検索**（`/search`）、**履歴・過去版の取得**（削除済みも可）、**差分同期用の変更一覧**（`/changes?since=`）、
**同時編集の競合検出**（`version` / `if_version` → 409）、**自動コミット**、ブラウザアプリ向けの**厳格な CORS**（完全一致オリジン＋トークン必須）。

```python
from lawgit_client import LawGitClient
c = LawGitClient("http://127.0.0.1:8787")
c.edit_rule(uuid, lambda r: {"body": r["body"] + "\n- 追記"})   # 競合しても自動リトライ
for commit in c.watch(): print(commit["subject"])                # 新しい commit を追従
```

メソッド一覧・リクエスト例・安全設計は [docs/API.md](docs/API.md) を参照。

## e-Gov からのインポート（独立した Python スクリプト）

ネットワーク通信を伴う処理は危険なため、**Rust 製の lawgit 本体には一切含めていません**。
取り込みは `tools/egov_import.py`（Python 3.9+、標準ライブラリのみ）が単独で行い、書き込み先は `laws/<UUID>.xml` だけです。

```sh
python3 tools/egov_import.py --repo legal-data 415AC0000000057            # dry-run（既定。何も書かない）
python3 tools/egov_import.py --repo legal-data 415AC0000000057 --write    # 書き込み
python3 tools/egov_import.py --repo legal-data --from-file law.xml --law-id 415AC0000000057 --write  # 通信なし
python3 tools/egov_import.py --repo legal-data 335AC0000000105 --asof 2025-04-01 --write --update
lawgit --repo legal-data validate && cd legal-data && git diff   # 確認してから自分で commit
```

- 通信先は `https://laws.e-gov.go.jp/api/2/law_file/xml/{法令ID}` のみ（他ホストへのリダイレクトは拒否、サイズ上限・タイムアウトあり）。
- DOCTYPE / ENTITY を含む XML は解析前に拒否。取得データは実行・評価しない。
- 既に取り込み済みの法令は `--update` を付けない限り上書きしない。UUID は既存のものを保持（不変）。自動 commit はしない。
- 変換: `LawNum`→`law-number`、`LawType`→`type`（Act→法律 など）、`LawTitle`→`name`、`LawBody` 配下→`<body>`（e-Gov の要素語彙のまま構造保持）。
### 全法令の一括インポート

```sh
python3 tools/egov_import.py --repo legal-data --all                              # 件数確認のみ（dry-run）
python3 tools/egov_import.py --repo legal-data --all --max 5 --write --yes        # まず 5 件で試す
python3 tools/egov_import.py --repo legal-data --all --write --yes --delay 1.0    # 全件（数千リクエスト＝数時間）
```

- 一覧 API（`/laws`）をページ送りして全法令の ID を収集し、順に取り込みます。`--all --write` は誤爆防止のため `--yes` が必要です。
- 取り込み済みの法令は自動でスキップするので、中断しても再実行で続きから再開できます。10 回連続で失敗したら中止します。
- `--delay`（既定 1 秒、最小 0.5 秒）で公開 API への負荷を抑えます。

### 過去の改正履歴を Git に自動復元（`--history`）

```sh
python3 tools/egov_import.py --repo legal-data 415AC0000000057 --history           # 計画の表示のみ
python3 tools/egov_import.py --repo legal-data 415AC0000000057 --history --write   # commit を作成
python3 tools/egov_import.py --repo legal-data --all --history --write --yes       # 全法令の全履歴
```

各法令の改正版一覧（`/law_revisions/{法令ID}`）を取得し、版ごとの XML を **施行日の古い順** に commit します。

- **1 改正法 = 1 commit**: 同じ改正法・同じ施行日で複数の法令が変わった場合は、影響する全 XML を同じ commit に含めます（規格 7 章・README の運用どおり）。
- commit 日時は施行日（JST 0:00）です。1970 年より前は Git が表現できないため 1970-01-02 に丸め、実際の日付は commit メッセージ（`Enforced:` / `Promulgated:`）に残します。
- commit メッセージ末尾に `Amending-Law:`（改正法令番号・ID）と `E-Gov-Revision:`（版 ID）を記録します。これを手掛かりに、再実行時は復元済みの版をスキップします（冪等・再開可能）。
- 作者は固定の `e-Gov Import <egov-import@lawgit.invalid>`。committer 日時も施行日に揃えるため、同じ入力なら同じ内容の履歴になります。
- 既定では未施行の将来版は除外します（`--include-unenforced` で含める）。最終状態は現行施行版になります。
- 改正版一覧は `.git/lawgit-egov-cache/` にキャッシュします（作業ツリーは汚しません。`--refresh` で無視）。
- **安全のための前提**: 書き込み時は、リポジトリ最上位であること・ステージ済み変更が無いこと・`laws/` に未コミット変更が無いことを確認し、満たさなければ何も変更せず中止します。commit は `--no-verify`（フック無効）で行い、メッセージは標準入力経由で渡します。取得したメタデータは制御文字を除去します。
- 注意: 中断後に再実行すると、取得に失敗して後回しになった版は、より新しい commit の後ろに追加されます（日付は施行日のまま）。
- 注意: 履歴データの範囲は e-Gov の収録範囲に依存します（v2 は平成 29 年 4 月 1 日以降のデータに対応）。

- 相互検証: `--write --verify`（`--lawgit-bin PATH`）を付けると、書き込み後に Rust 側の `lawgit check` で全ファイルを再検証します（`--history` 併用可）。
- テスト: `python3 tools/test_egov_import.py`（オフライン）。
- 注意: e-Gov API への実通信は、この開発環境では確認できていません。初回は dry-run、または `--from-file` で試してください。

## コマンド一覧

| コマンド | 目的 |
|---|---|
| `init` | laws/ rules/ を作成し git init |
| `new` | 法令 XML を UUID v4 付きで作成 |
| `validate` | laws/ の検証（構文・必須要素・UUID 形式・ファイル名一致・重複・本文構造・LF） |
| `list` / `show <UUID>` | 一覧 / XML 表示 |
| `check` | laws/ + rules/ の総合検証 |
| `rule make/new/list/show/set/edit/path/delete/validate` | ルール管理 |
| `commit -m` | 検証付き commit |
| `config autocommit [on\|off]` | 自動コミットの設定 |
| `search <語句>` / `history <UUID>` | 全文検索 / 変更履歴 |
| `serve` / `rpc` / `openapi` | 連携窓口 / OpenAPI 出力 |

`--repo` で対象リポジトリを指定（既定はカレントディレクトリ）。終了コード: 0 成功 / 1 検証エラー / 2 使用方法・実行エラー。

## 今回のスコープ外

改正文の自動解析、GUI エディタ、条・項・号の要素語彙の完全な定義（本文は構造化 XML として保持するが語彙は別途仕様化）。
