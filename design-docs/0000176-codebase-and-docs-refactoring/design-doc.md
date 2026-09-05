# コード・ドキュメントの簡素化と障害時の整合性改善

**Status**: In Progress
**Progress**: 0/36 tasks complete
**Last Updated**: 2026-09-05

## Overview

コードとドキュメントのレビューで得た20件を、再現済みの4不具合を優先する段階的な改善にまとめる。外部契約を明示したうえで、プロセス・DB・pane・インストールの失敗処理、Rustの内部境界、WebUIの取得処理、文書の参照構造を簡素化する。本書は日本語の設計提案であり、実装・コミット・リリースの承認を表さない。

## Success Criteria

- [ ] F01–F04それぞれに修正前の失敗を検出する回帰テストがあり、修正後に成功する。
- [ ] 同一fleetにactive monitorを2件登録できず、既存重複の移行失敗ではレコードとpaneが保存される。
- [ ] pane・runtime・assetsの各失敗点で後始末を検証し、一次エラーを後始末エラーで置き換えない。
- [ ] 意図的変更一覧以外のCLI/HTTPの形・順序・null・終了コード・エラー文字列が契約テストを通る。
- [ ] WebUIのmember履歴取得が各201行以内になり、初回エラーと空履歴を区別し、fleet切替前の応答が新画面を更新しない。
- [ ] インストール済みskillsの必須ローカル参照がMarkdown link・inline-code表記ともcheckoutなしで解決し、bootstrap例が4識別ラベルと実在するroleへの参照を持つ。
- [ ] SPECの既存構造と再実装に必要な契約詳細が保持され、今回特定した仕様差分とsupervision/recoveryの矛盾が解消する。
- [ ] F01–F20の対応表にある全作業と検証が完了し、Rustテスト・lint、adminテスト・lint・build、docs buildが成功する。

---

## Background

対象はRust版CAFleet 0.24.4、DB schema 7、React製admin、`docs/`・`SPEC.md`・`skills/`・プロジェクト規則である。レビュー時の480 Rustテスト、admin build/lint成功は既存の基準値であり、本書の将来の実装を検証した結果ではない。F01–F03は一時Rust probe、F04は実際のTSグループ化関数を抽出したprobeで再現されている。F04のブラウザ画面での再現は未実施である。

詳細なレビュー入力は[review-brief](.notes/review-brief.md)。以下の仕様と検証条件は同ファイルを開かず実装できるよう記載する。作成前の質問に対するDirector回答（CAFleet message 894）は本提案の設計判断として採用したもので、ユーザーによる追加承認とは扱わない。

| 既存設計書 | 本書との境界 |
|---|---|
| `0000170-docs-skills-affirmative-simplification`（Complete） | 文書の所有場所、肯定的表現、overlayの自己完結性、異なる読者向けRequired-readingを維持する。今回の新しい不具合・インストール後のリンク切れ・通常のユーザーフィードバック受付を追加対象にする。 |
| `0000174-fix-codex-monitor-lifecycle`（Complete） | Codexがmanaged execution sessionを保持し、起動ログを確認してから`monitor live`を送る契約を維持する。Rust runtimeの失敗時解放は別の修正である。 |
| `0000175-notification-failure-reporting-and-command-isolation`（Complete） | 永続化済みmessageの通知失敗・再送禁止・one-shot CLI単独呼出し・非同期handoffを維持する。notifier移動で成功扱いに戻さない。 |
| `0000175-reliable-pane-message-notifications` | ディレクトリを確認したが`design-doc.md`はなく、`.prompts/`のみ存在する。未成立の仕様を実装済みの契約として扱わない。 |

---

## Specification

### 1. 対象と変更管理

実装は各Stepの対象文書を先に更新し、次に回帰テスト・コードを変更する。文書順序は`docs/`、必要な`README.md`、`SPEC.md`、対象`skills/`、規則とする。`docs/`は説明の主たる置き場、`SPEC.md`は再実装の権威ある契約であり、リンクだけの要約へ縮めない。インストール先の`~/.codex/skills`等を実装対象にせず、リポジトリの`skills/`を変更する。

| ID | 問題・主要な根拠 | 対応Step |
|---|---|---|
| F01 高・再現済み | `cli/system.rs::SystemRunner::run`がpipeを読み始める前に子終了を待つ | 1 |
| F02 高・再現済み | `cli/member.rs`の事前チェックと`broker/members.rs::register_member`の間でmonitor登録が競合 | 2 |
| F03 高・再現済み | `multiplexer/herdr.rs::split_window`のrun失敗で作成paneを失い、CLIの補償もsend_exit | 3 |
| F04 中・アルゴリズム再現済み | `broker/queries.rs::list_timeline`がsummaryを返し、`Timeline.tsx`が架空の宛先/ACKを数える | 4 |
| F05 | `get_member`の13要素tuple→JSON→再parse、各consumerのexpect/clone | 5 |
| F06 | `webui/mod.rs`から`cli::helpers::CliNotifier`へ依存 | 5 |
| F07 | helpers/doctor/setupのschema/assets診断と接続が重複 | 6 |
| F08 | `roster_rows`の未使用相関集計、`get_member_names`のIDごとのSQL | 6 |
| F09 | MemberDetailが全履歴を取得後201行へ切り詰め | 7 |
| F10 | createの長引数・手動rollback、bootstrapの長時間write transaction、monitor claim後の早期return | 3・8 |
| F11 | capture/monitor scanのANSI・日時・SHA256生成が重複 | 8 |
| F12 | global fleetId・URL・App stateの重複と複数load owner | 9 |
| F13 | Timeline/MemberDetailのcatchが初回障害を空表示にする | 9 |
| F14 | `assets.rs`が既存ツリーを削除してからcopy、失敗後も同versionの健康な記録が残る | 10 |
| F15 | SPEC/cli-options等の重複と並び順・activity・join・配布方式・formatter説明の差分 | 6・11 |
| F16 | recoveryのidle/unreadヒューリスティックがsupervisionのcapture判定と矛盾 | 11 |
| F17 | checkoutでは有効な24リンクとoverlayの4つのinline-code参照がインストール後にskills外へ逸脱 | 11 |
| F18 | 243行のquickstartと情報不足のmonitor prompt | 11 |
| F19 | 共通プロトコルの反復、ユーザーにCOMMENT構文を要求 | 11 |
| F20 | prose固定テスト、フロントテスト不足、CI重複、mise引数例の誤り | 4・9・12 |

データ保持期間の変更、履歴削除、通知の自動再送、新しい管理CLI、全面的なモジュール再編、大規模なfetchライブラリ導入、一般的なCLIのpanic/BrokenPipe対策は対象外とする。timelineの行数制限によるbroadcastグループの途中切れはF04と別問題であり、APIのページング意味論は変更しない。

| 意図的に変える外部挙動 | 変化 |
|---|---|
| F01 | pipe容量を超える正常な出力が偽のtimeoutにならない。 |
| F02 | 競合による第2monitorをDBで拒否し、既存重複の移行を診断付きで停止する。 |
| F03/F10 | 作成失敗時に所有paneをkillし、後始末失敗も報告する。bootstrap subprocessに期限を設け、runtimeの初期化失敗でもclaimを解放する。 |
| F04 | timelineからsummaryを除外し、配送だけを表示・集計する。 |
| F09 | inbox/sent HTTPに任意`limit`を追加する。省略時とCLI履歴は従来どおり。 |
| F12/F13 | 重複取得・古い応答の反映を防ぎ、通信失敗を明示する。 |
| F14 | 不完全なassets交換を健康扱いせず、setupで回復可能にする。 |
| F15境界修正 | activityの将来日時による負の`idle`だけ0へ補正する。4件の再現済みバグとは別の追加修正。 |
| F19 | ユーザーの通常の文章をDirectorが内部COMMENTへ変換する。ユーザーに構文入力を求めない。 |

### 2. プロセス出力・期限・後始末（F01）

`CommandRunner::run(argv, timeout_secs) -> Result<String, RunError>`を維持する。`Some(timeout)`ではspawn直後からstdout/stderrを並行して排出し、子プロセスの生存確認とdeadline確認を同じループで行う。実装は既存`nix`依存の安全なnonblocking FD/poll APIを使い、必要な`fs`/`poll` featureを追加する。read専用threadを無制限joinする方式は採らず、所有するpipeの終了を呼出し内で管理する。

1. spawnした直接の子と両pipeを所有し、両FDをnonblockingに設定する。失敗した場合も子をkill/reapする。
2. 各反復でdeadlineと`try_wait`を確認し、両streamを公平に読み取る。片側の連続出力がもう片側やdeadlineを飢餓状態にしないよう、1反復あたり各64KiBを上限にする。poll待ちは20msと残り期限の短い方とする。
3. 子の終了かつ両streamのEOFで完了する。成功は従来どおりstdoutのlossy UTF-8、非0終了はstderrのlossy UTF-8を`Failed`へ渡す。出力量の切り詰めを新設しない。
4. 期限到達時は直接の子をkillしwaitで回収、両read FDを閉じ`Timeout`を返す。子終了後も子孫がpipeを保持するケースではdeadlineまでにEOFがなければ同じ扱いとする。子孫全体の停止は保証しない。
5. read/poll/try_waitエラーでもkill/reapとFD解放を行う。一次原因を保持し、kill/reap失敗は付随診断にする。signal割込みは期限を再計算して再試行する。`None`は従来の期限なし`wait_with_output`経路を使える。

期限は観測・停止開始の上限であり、OSが応答しない場合まで厳密なwall-clock復帰時間を保証しない。通常の子では1秒timeoutが5秒以内に復帰する統合テストを設ける。正常系は1MiB stdout、1MiB stderr、両方同時の各ケースを余裕のあるdeadlineで検証し、出力欠落・zombie・残存readerがないことを確認する。実際のsleep timeout、非0終了、空出力、FD設定/読み取り失敗、子終了後にpipeを保持するfixtureも検証する。

### 3. active monitorの一意性と移行（F02）

schema 7に続くmigration（現時点では`V8__unique_active_monitor.sql`）に以下を追加する。着手時にheadが進んでいれば次の連番を使用する。

```sql
CREATE UNIQUE INDEX idx_members_one_active_monitor_per_fleet
ON members(fleet_id)
WHERE status = 'active'
  AND json_extract(member_card_json, '$.cafleet.kind') = 'monitor';
```

判定は既存`active_monitor_member_id`と一致させる。通常member・deregistered monitorは制約対象外、fleet間は独立である。INSERTだけでなくstatus/card/fleet_id更新にも制約が効く。root Directorのcard生成はmonitor markerなしを維持し、表示用kindの「director優先」は変えない。

CLIの事前チェックはエラー順序と早期診断のため残すが、正しさはDBに依存させる。登録transactionは`IMMEDIATE`でwriterを直列化し、monitorの場合はtransaction内でも既存IDを確認する。一意性違反は当該制約に限って`ActiveMonitorExists { fleet_id, member_id }`へ変換し、CLIは既存文字列`fleet {fleet_id} already has an active monitor member (member {existing})`、exit 1を維持する。他のSQLエラーをこのエラーに偽装しない。競合敗者は登録・placement・pane作成に進まない。

移行前診断は`setup`のDB半分において、schemaの未version/新しすぎる拒否の後、migration実行前に行う。既存のmembersがある場合は同じpredicateで重複fleetとmember IDを昇順取得する。重複時は`active monitor duplicates prevent migration: fleet <id>: members <ids>; ...`を表示しDB半分を失敗させ、レコード・pane・schema historyを変更しない。新規DBでは診断対象がないので通常移行する。

診断とDDLの間の競合も制約作成で拒否される。pending migration群はrefineryのgrouped transactionで適用し、失敗時に旧schema/historyへ戻るよう保証・試験する。制約作成失敗後に重複を再取得できれば同じ診断を出し、別の原因なら元のmigrationエラーを維持する。既存migration本文の書き換えや自動的な生存者選択は行わない。

既存重複の回復手順は次のとおり。

1. 対象DBに接続する新規登録処理を止め、表示されたIDから運用者が残すmonitorを選ぶ。
2. 旧schemaに対応する直前のリリースを別パスから実行し、同じ`CAFLEET_DATABASE_URL`と設定ディレクトリを使う。新CLIの`member delete`はbehind-schema guardに阻まれるため用いない。
3. setupはDB/assets独立実行の契約を維持するため、失敗した新setupがassetsだけ更新済みの場合がある。この場合は旧バイナリの`setup`で同じ対象backendのassetsを旧版へ戻してから、旧バイナリの`member delete <surplus-id>`を1呼出しずつ行う。DBは旧headなのでこの経路を実行できる。
4. 重複解消後、新バイナリの`setup`を再実行する。schema成功後は旧版へ自動downgradeしない。

2接続がともに事前確認でNoneを観測→A登録commit→B登録という決定的interleavingをテストする。migrationは空DB、非重複のpopulated DB、重複DB、適用済み再実行、diagnostic後の競合で検証し、失敗後のschema/history/行数とpane操作0回をassertする。

### 4. pane所有権と作成補償（F03・F10の一部）

`split_window`の成功復帰までbackendが新paneを所有し、成功後はCLIの作成処理へ所有権を渡す。Herdrはsplit応答からIDを得た直後にguardを作り、後続のrun失敗で`kill_pane(id, true)`を呼ぶ。pane close失敗時もIDと一次runエラーを保持して返す。ID取得前の壊れたsplit応答など、外部がpaneを作成したか確認できない場合は「ID不明で補償未確認」を診断し、他paneを推測で閉じない。

| 失敗点 | guard所有者と補償順序 | DB側の最終処理 |
|---|---|---|
| member登録失敗 | brokerが登録transactionをrollback、pane guardなし | 行を追加しない |
| memberのplaceholder展開失敗 | CLIの登録guardのみ、pane未作成 | 登録をderegister、placementを既存規則で除去 |
| memberのHerdr run失敗 | backendのpane guardがkillを試行→エラーをCLIへ返す→CLIの登録guardがderegister | deregister/placement除去 |
| member placement更新Err/None | 更新SQLの失敗処理後、CLIのpane guardがkill→登録guardがderegister | deregister/placement除去 |
| fleet callbackのpane作成前失敗 | pane guardなし→brokerがbootstrap transactionをrollback | fleet・Director・monitor・placementの追加を取消 |
| fleet callback内のHerdr run失敗/timeout（ID既知） | backendのpane guardがkillを試行→callbackがErr→brokerがbootstrap transactionをrollback | 同上 |
| fleet callback成功後のplacement INSERT/commit失敗 | CLIがcallback成功時に受領したpane guardを保持→brokerがtransactionをrollbackしてErrを返す→CLIがkill | 同上 |
| fleet callbackがID取得前に失敗/timeout | backendが補償未確認を返す→brokerがbootstrap transactionをrollback。paneを推測してkillしない | 同上、pane側は未確認の診断を保持 |

backendが補償を試した失敗は`PaneCleanup::Attempted { pane_id, error: Option<_> }`、ID不明は`PaneCleanup::Unknown`としてCLIへ返す。CLIはAttemptedに対して再killせず、残ったDB補償を行う。callbackは`split_window`成功後すぐCLIのguardへ所有権を渡してIDを返し、その間に別の失敗しうる処理を挟まない。fleetのpost-callback失敗ではbrokerのtransactionを閉じてからpane補償するため、pane-firstを要求する新しいbroker APIは導入しない。

補償順序は上表に従い、先行する補償の失敗でも残りを試す。`send_exit`はshell/pane終了の保証にならないため作成rollbackから除く。cleanup失敗は一次エラーの後に`cleanup failed for pane <id>: <detail>`、`cleanup failed for member <id>: <detail>`、`cleanup failed for fleet <id> transaction: <detail>`を追加し、成功や完全rollbackと誤報しない。transactionのrollback失敗も明示診断し、未確定のDB状態を完全取消と断言しない。二重kill/deregisterを防ぐため明示的`finish`/`rollback`でguardをdisarmし、Dropは未処理の最終防御だけにする。memberのplacement確定またはfleetのcommit成功時には、既存`emit`を呼ぶ前に全作成guardをdisarmする。text/JSONの既存出力境界は変更せず、新しいstdout失敗の終了コード・診断契約は導入しない。

FakeRunner/FakeMuxとtransaction境界のevent fixtureで上表の各順序、list→split(ID取得)→run失敗→close、close/rollback失敗の診断、成功時killなしを検証する。既存の正常な`member delete`や通知keystrokeの振る舞いはこの変更に混ぜない。

### 5. timelineと履歴HTTP（F04・F09）

timeline SQLのWHEREに`g.type='unicast'`を追加し、owner memberによるfleet scopeと`ORDER BY status_timestamp DESC, message_id DESC LIMIT 200`を保持する。summaryはDB・message show・broadcast結果には残す。summaryを配送扱いする誤りだけを直し、APIから返る配送の順序とUIの既存created_at表示順は混同しない。

TSは実際のwireにある`type`を宣言し、`FormattedMessage`を`type: 'unicast'`（宛先ID/nameは非null）と`type: 'broadcast_summary'`（両方null）のunionとして表現する。timeline・inbox・sentは配送型へnarrowして描画する。`groupMessages`を純粋関数として`timeline.ts`へ移し、非nullの`origin_message_id`を明示判定する。summaryが混入した入力も防御的に集計から除外する。

2つのpending配送と1つのcompleted summaryを与えて宛先2・ACK0になることをSQL、HTTP、TSでそれぞれassertする。1配送をACKすると宛先2・ACK1、全ACKで2、通常unicast、空配列も検証する。行limitでbroadcastが途中までになる場合、集計は「取得できた配送」に対するものとし、全配送の完了率と断言しない。ReactionBarの説明にこの限定を記載し、limitをgroup単位へ変更しない。

| HTTP項目 | 契約 |
|---|---|
| 対象 | `GET /api/members/{member_id}/inbox`、`/sent` |
| 新query | 任意`limit`、1–1000の10進整数。空・0・負・小数・非数字・overflow・重複指定は422 `{"detail":"limit must be an integer between 1 and 1000"}`。未知queryは既存どおり無視。 |
| 省略 | 全件を返す既存動作。CLIのinbox/sent相当の取得も変更しない。 |
| 順序 | `status_timestamp DESC, message_id DESC`。同timestampでも決定的。ここでの「最新」はstatus更新順。 |
| envelope | `{"messages":[...]}`、各rowのキー順序・名称・nullを維持。cursor/has_more/totalを追加しない。 |
| validation順序 | 既存Path抽出、fleet header（400）、fleet存在（404）、member所属（404）を維持し、その後limit検証。SQLエラーは既存500 detail形式。 |
| broker | `HistoryOptions { limit: Option<usize> }`をinbox/sent取得へ渡す。指定時だけSQLにbindしたLIMITを適用し、取得後truncateで代替しない。 |
| WebUI | 両endpointに`?limit=201`。各最新200行を表示し、201行目があれば既存の省略表示を明示する。200行以下なら省略表示なし。 |

明示limit付きHTTPとWebUIの取得行数を制限する設計であり、互換性のため残す無制限HTTPや保存容量を制限したとは主張しない。0/200/201/大量行、同timestamp、他fleet、deregistered member、各不正limit、省略時201件超の互換性を検証する。既存indexでのquery planを確認し、今回不要なindex追加はしない。

### 6. Rust内部型と依存方向（F05–F08・F15）

SQLから内部の小さな型へ一度だけ変換し、CLI/HTTP presenterでwireにする。型移行はmember→placement→message→monitorの呼出し単位で進め、互換adapterを短期間残しても全consumer移行後に除去する。汎用repository frameworkは作らない。

| 内部型 | 主なfieldと制約 |
|---|---|
| `MemberRecord` | `member_id: i64`, `fleet_id: i64`, `name/description/registered_at: String`, `status: MemberStatus`, `kind: MemberKind`, `skills: Vec<Value>`, `placement: Option<Placement>`。自由形式skill要素だけValueを許す。 |
| `Placement` | `backend`, `mux_session`, `mux_window_id`, `coding_agent`, `created_at`: String、`mux_pane_id: Option<String>`。placementなしとpane未確定を区別。 |
| `MessageRecord` | ID類:i64、`to_member_id/origin_message_id: Option<i64>`、`kind: MessageKind`、`status: MessageStatus`、`created_at/status_timestamp/text: String`。summary宛先nullを保持。 |
| `MonitorRuntime` | `fleet_id: i64`, `pid: Option<i64>`, `started_at: Option<String>`, `last_tick_at: Option<String>`, `last_wake_at: Option<String>`, `wake_requested_at: Option<String>`, `tick_seconds: i64`, `wake_interval_seconds: Option<i64>`。fieldごとの意味は下表。 |
| `CaptureSnapshot` | `content: String`, `captured_at: String`, `content_sha256: String`。member/pane metadataは呼出し側が付与。 |
| `Diagnosis` | `SchemaState`（Missing/Unversioned/Behind/Head/Ahead/Unreachable）とbackend/pathごとの`AssetState`。表示文言を持たない。 |

| MonitorRuntime field | NULL・0とライフサイクルの意味 |
|---|---|
| `fleet_id` | 非nullのfleetキー。row不在は`Option<MonitorRuntime>::None`で表現し、0を代用しない。 |
| `pid` | NULLはclaim未保持。claim時に直接のloop PIDを記録し、正常clearでNULLへ戻す。0を未保持として新たに解釈せず、既存process probeの判定を維持。 |
| `started_at` | NULLはclaim開始時刻なし。claim/reclaimで更新し、正常clearでNULL。 |
| `last_tick_at` | NULLまたはparse不能はheartbeatがfreshでない。claim/tickで更新し、正常clearでNULL。 |
| `last_wake_at` | NULLは成功wakeの記録なし。scheduled/forcedの配送成功時だけ更新し、clear/reclaimを跨いで保持。NULLならcadenceの基準はstarted_atへfallback。 |
| `wake_requested_at` | NULLはforced-wake要求なし。要求の上書きでcoalesceし、配送成功またはreclaimでNULL。正常clear単独では消さない。 |
| `tick_seconds` | 非null、DB default 5、正常なCLI入力は正値。0はdisableではなく不正入力として既存CLI検証で拒否。正常clearで保持。 |
| `wake_interval_seconds` | NULLはV5追加前のrowが未再claimで値未記録という状態であり、0へ変換しない。0は定期wake無効（forced wakeは可能）、正値は秒間隔。claim/reclaim/PATCHで記録し正常clearで保持。 |

loopのclaim後はwake_interval_secondsがSomeになる不変条件を維持する。DB rowのnullable型とHTTPの停止時projectionを混同せず、停止時に隠す時刻と保持するintervalは既存presenter契約で変換する。型移行の回帰ではV5由来NULL、0、正値、clear/reclaim、forced-wakeの各caseを検証する。

内部enumは不正DB値を`InvalidStoredValue`へ変換し、panicを境界まで漏らさない。既存card_skillsの壊れたJSON/skills不在→空配列というfallbackは維持する。時刻文字列のparse/formatを型導入だけで変えない。JSON key orderは`serde_json`のpreserve_orderに依存する既存出力なので、各presenterが明示順序で構築する。

`SystemRunner`・`SystemProbe`と実プロセスのnotifier adapterをCLIより下の`runtime/`へ移す。依存方向はCLI/HTTP→runtime adapter→brokerの通知trait・multiplexer/coding_agent。brokerはプロセス起動もHTTPも知らず、webuiはcliをimportしない。`NotificationAttempt`、永続化済みmessage ID、transport診断、既存設計0000175のCLI部分失敗出力を保持する。

domain errorは`ActiveMonitorExists`、`MemberNotFound`、`FleetNotFound`、`InvalidStoredValue`等、実際に分岐に必要なvariantだけ導入する。CLIでUsage/App/Valueの既存終了コード・文字列へ、HTTPで既存status/detailへ写像する。欠損sender/nameをpanicする箇所は整合性エラー500へ変え、存在しない名前を捏造しない。

共通診断は同一invocationで使用するConnectionを受け取り、schema→assetsのguard順を保持する。doctorは接続失敗も報告値として扱い全セクションを表示、通常CLIは必要なguardで停止する。setupはDB失敗でもassets半分を試す契約を維持し、DB作成/移行後は同じ接続で診断し直す。初回接続が失敗していた場合の再接続まで「1接続」に固執しない。HTTPは従来どおりblocking handlerごとに接続する。

`list_roster`はmessage activityを計算しないlean queryとし、message holder包含のEXISTS条件・kind・placement・member_id順を維持する。`list_members`はactivity付きqueryを使う。`get_member_names`はIDを重複排除し、最大500 IDずつbindしたIN queryで取得する。空入力はSQL0回、未知IDはmapから欠落、deregisteredは含み、戻り値はBTreeMapとする。SQL回数は`ceil(unique_ids/500)`以下で、文字列連結するのはplaceholderだけとする。

仕様差分は以下を採用し、動作固定テストとSPECを同時にそろえる。

| 項目 | 採用する契約 |
|---|---|
| fleet並び順 | 現行Rustどおり`created_at DESC, fleet_id DESC`。SPECの同時刻ASCを訂正。 |
| `last_sent` | sender一致の全message（summary含む）の`MAX(created_at)`。status更新日時へ変更しない。 |
| `last_recv` | owner一致・unicastの`MAX(created_at)`。 |
| `last_ack` | owner一致・unicast・completedの`MAX(status_timestamp)`。 |
| `idle` | 3時刻の最大値を選び既存parse規則で解釈、全部null/parse不能はnull。計算結果だけ`max(0, seconds(now-latest))`へ補正。 |
| timeline scope | owner memberとのjoin。sender joinというWebUI specを訂正。 |
| assets配布 | Rust binaryに同梱したskills/presetsをoffline展開。古いrelease archive取得説明を更新。 |
| spawn formatter | Rust mini formatterの4placeholderとliteral brace escape契約を説明する。Python `str.format`全機能を約束しない。 |

### 7. lifecycleとcaptureの共通処理（F10・F11）

`MemberCreateOptions`と`FleetCreateOptions`に現在の引数を集約する。検証順は既存の各CLI契約に従い、同じように見えるfleet/memberの異なる前提確認を無理に共通化しない。共有するのはspawn準備、pane所有guard、補償結果の構築である。

fleet bootstrapはfleet/Director/monitorの一括commitを維持する。外部副作用をtransaction外へ移して未完成fleetを公開する方式は採らない。DB transaction前にprompt読込・backend検証・cwd/env/argv準備の可能な部分を済ませる。ID依存の展開とpane spawnのみcallbackに残し、callbackのsubprocess全体に単調時計の30秒deadlineを渡す。Herdrのlist/split/run/resize、tmuxのsplit/layoutの各呼出しは残り時間で制限し、同じ30秒を毎回リセットしない。補償killは別の5秒budgetとし、既存DB busy timeout 5秒は維持する。DB lockの待ち時間やOS停止まで30秒以内と保証しない。timeout時のguard所有者と補償順序は§4の失敗経路表を用いる。特にcallback内run失敗はbackendのkill試行がDB rollbackより先、callback成功後のplacement/commit失敗はDB rollbackがCLIのkillより先となる。

monitor loopはclaim成功直後に`MonitorLease`を所有する。signal registerの各成功handleを保持し、次のsignal登録失敗、startup write/flush失敗、tick失敗、正常停止、所有権喪失の全経路で解除する。`clear_monitor_runtime(fleet_id, pid)`の条件付き更新を維持し、別PIDの新ownerを消さない。主処理エラーとclearエラーが両方あれば前者を保持し後者を付記、主処理成功・clear失敗だけならclear失敗で終了する。SIGKILL/crash時は既存stale reclaimが回復経路である。

captureは`CaptureSnapshot::from_raw(raw, ansi, now)`へ集約する。ansi=falseは既存strip_ansi（CRの正規化を含む）後のcontent、trueは受領したrawを使用し、その最終contentのUTF-8 bytesをSHA256の小文字hexへ変換する。時刻形式、lines入力、scanのerror時null、text出力を保持する。scanのtext見出しはtext presenterでのみ作り、JSON経路では構築しない。

signal登録/Write/flush/clearを注入可能にして各早期returnを検証する。captureはANSI、CRLF、単独CR、Unicode、空入力でcapture/scanのcontentとhashの一致を検証し、timestamp差は固定時計で除く。

### 8. WebUIのfleet状態と非同期状態（F12・F13）

URL routeを選択fleetの唯一の情報源にし、`createFleetClient(fleetId)`でfleet IDを閉じ込める。`listFleets`は非scope client、残りは明示fleet clientをprops/contextで受け取る。global `setFleetId`を削除し、fleet選択handlerはnavigateのみ行う。

Appはrouteとfleet存在確認、Dashboardはmember roster・monitorとrefreshKey、Timeline/MemberDetailはそれぞれの履歴取得を所有する。AppからのinitialMembers事前取得はなくし、同じrosterをroute effectとDashboard mountの両方から取得しない。Dashboardはfleet IDでkeyを付け、member panelもmember IDでkeyを付ける。各resourceの初回取得は1経路にする（開発StrictModeのabort済み試行は許容）。

取得はAbortControllerと世代IDを持ち、unmount・fleet/member変更でabortする。レスポンス反映とfinallyのinFlight解除は現在の世代だけが行う。更新中にrefreshKeyが増えたら最新の1回をpendingとしてまとめ、完了後に再取得する。失敗/abortでもguardが残ってpolling不能にならないようにする。5秒pollingとvisibilityに依存しない既存仕様、手動Refresh・送信成功後の更新経路を維持する。

| resource状態 | 表示・再取得 |
|---|---|
| 初回loading | skeleton |
| success・空 | No messages |
| 初回error | エラー文と再試行ボタン。空表示を出さない。 |
| refresh中 | 同じfleet/memberの既存データを保持。 |
| refresh error | 既存データ＋更新失敗表示＋再試行。 |
| abort/旧世代 | 表示を更新しない。 |

不正/不存在/削除済みfleetは既存のfleet一覧への遷移、deep linkと戻る/進むを維持する。通信障害は不存在と区別し、routeを維持して再試行できる画面を出す。以前のfleet名・宛先が新fleetの送信フォームに混在しないことを確認する。

minimal frontend test環境は既存Vite系に合わせたVitest、必要なDOM fixture用jsdomとReact Testing Libraryとし、実装時のlockfile互換性を確認する。純粋なgroupingテストに加え、遅延promiseでA→B切替の逆順応答、初回失敗→再試行、refresh失敗のstale表示、連続refreshのcoalesce、unmount中完了、各mountでroster取得1回を検証する。fetchライブラリの移行は行わない。

### 9. assetsのstage・swap・回復（F14）

成功済みインストールと記録を維持しながらbackend単位で置換する。2 skills・preset・削除対象の旧cafleet-researchを同じinstall planで管理する。他backendの既に成功した更新は巻き戻さない。複数ディレクトリとSQLiteを跨ぐ単一atomic commitとは説明しない。

stage/backupは各targetと同じ親ディレクトリに一意な隠し名で作る。対象外のskillsを含む親全体はrenameしない。targetがsymlinkの場合はentry自体を退避しリンク先を辿って削除しない。異なる設定pathが同じ物理skills treeを指す場合も含め、target pathの正規化順にOS advisory lockを取得して同時setupを直列化する。lockはプロセス終了で解放し、PID fileだけで判定しない。

1. 全ファイルをstageへ書き、embedded manifestと必須参照を検証する。copy/検証失敗はstageだけ除去し、現行ファイル・asset_installsを変更しない。
2. assets identity path配下の`.cafleet-install-journal.json`に、transaction ID、phase、各target/stage/backupのpathと旧存在状態、前のasset_installs行（存在しない場合null）、新versionとmanifestを記録する。journalはtemp→renameで更新し、変更前にflush/syncする。
3. 各targetをbackupへrenameしstageをtargetへrenameする。各操作前後の状態をjournalに記録する。cafleet-research除去もbackupへの移動として扱う。途中失敗は逆順に旧entryへ復元する。
4. 全swap成功後に`record_asset_install`をtransactionでcommitする。失敗ならファイルと旧記録を復元する。成功時はjournalを`committed`として永続化する。
5. committed後にbackup/stageを削除しjournalを最後に削除する。このcleanup失敗はインストール成功と分けて診断し、次のsetupで再実行可能にする。

| 中断・失敗地点 | 判定と回復 |
|---|---|
| journal前 | 現行は変更なし。未参照stageを次setupで整理。 |
| journalあり・未committed | guard/doctorは不完全なinstallと判定。setupはlock取得後、記録と実際のtarget/backup存在を照合し、旧entryと旧DB記録へrollbackする。 |
| DB commit後・journal committed前 | 同じ未committed回復で旧ファイルと旧DB記録へ戻す。version一致だけで成功と推測しない。 |
| committed・cleanup途中 | 新manifestとDB記録を検証してcleanupを続ける。rollbackしない。 |
| 復元失敗/壊れたjournal | journalとbackupを保存し、不完全状態を報告して停止。健康な旧versionと誤報しない。 |

guard/doctorは当該resolved installのjournalを確認し、未完了なら`incomplete assets install at <path>; run 'cafleet setup' to recover`とする。healthy pathの既存出力は保持する。回復中に再度中断しても同じjournalから処理できるよう、各restore/deleteを冪等にし、backupは復元またはcommitted確認前に削除しない。電源断に対する耐久性はfilesystemのsync/rename保証に依存することを記す。

同version再インストールを含め、stage write、1個目/2個目skill交換、preset交換、record失敗、rollback失敗、各journal phaseでの再起動、同時setup、別filesystemのpreset、symlink、旧research除去をfixtureで検証する。各ケースでファイルdigest・記録・journal・残存backupをassertし、実ユーザーの設定領域をテストで変更しない。

### 10. 文書の所有場所とインストール時の自己完結性（F15–F19）

| 内容 | 正本と他の場所の扱い |
|---|---|
| 機械的CLI引数表 | clap定義から生成した引数名/型/default/必須性を、cli-optionsとSPECの既存section内の限定blockへ反映する。説明・エラー優先順・text layoutは手書き契約として残す。 |
| 機械的schema表 | 全migration適用済みfixture DBのsqlite metadataからcolumns/null/default/indexを生成する。migration historyを手編集しない。 |
| 人間向け利用説明 | `docs/`。READMEは入口、SPECは契約詳細、skillsは実行時必須手順という分担を維持。 |
| 共通member/Director手順 | 既存`cafleet/SKILL.md`のbroker操作、`base-dir.md`の書込規則、`coding-agent-overlays.md`のbackend差分、`supervision.md`のDirector判定へ集約。roleは必要な読込順と差分を保持。 |
| 実行時必須の参照 | 下記29参照のうちL01–L28（24リンク＋4 inline-code参照）は必須として`skills/cafleet/reference/runtime/`へ同梱する。L29は任意の公開SPECリンク。正本は`docs/docs/`、runtimeは明示生成物とし手編集しない。 |

全29参照の置換を以下に固定する。初回レビューのMarkdown link 24件をL01–L24、overlayのinline-code参照4件をL25–L28、生成対象data-modelのSPEC参照をL29とする。L01–L28の参照元は`skills/cafleet/`相対、元targetは`docs/docs/`相対、置換先はインストール済み`cafleet/reference/`相対である。L29だけは参照元・元targetをrepo相対で記載する。実際のMarkdown hrefは参照元から置換先への相対pathで出力する。行番号は2026-09-05時点の照合用で、移動しても同じ参照を追跡する。L01–L28はCLI引数・出力・通知・lifecycleの判断に使う必須28件、L29は再実装者向けの任意1件とする。

| ID | 参照元 | 元target | 分類 | installed-tree置換先 |
|---|---|---|---|---|
| L01 | `SKILL.md:43` | `spec/cli-options.md` | 必須 | `runtime/spec/cli-options.md` |
| L02 | `SKILL.md:81` | `spec/cli-options.md#positional-subject-ids` | 必須 | `runtime/spec/cli-options.md#positional-subject-ids` |
| L03 | `SKILL.md:81` | `spec/cli-options.md#permissionsallow-coverage` | 必須 | `runtime/spec/cli-options.md#permissionsallow-coverage` |
| L04 | `SKILL.md:137` | `spec/multiplexer-backends.md#push-notifications` | 必須 | `runtime/spec/multiplexer-backends.md#push-notifications` |
| L05 | `reference/cli.md:3` | `spec/cli-options.md` | 必須 | `runtime/spec/cli-options.md` |
| L06 | `reference/cli.md:22` | `spec/cli-options.md#message-body-truncation` | 必須 | `runtime/spec/cli-options.md#message-body-truncation` |
| L07 | `reference/cli.md:28` | `spec/cli-options.md#output-shapes` | 必須 | `runtime/spec/cli-options.md#output-shapes` |
| L08 | `reference/cli.md:69` | `spec/data-model.md#broadcast-grouping` | 必須 | `runtime/spec/data-model.md#broadcast-grouping` |
| L09 | `reference/cli.md:69` | `spec/message-envelope.md` | 必須 | `runtime/spec/message-envelope.md` |
| L10 | `reference/cli.md:116` | `spec/cli-options.md#error-messages` | 必須 | `runtime/spec/cli-options.md#error-messages` |
| L11 | `reference/cli.md:125` | `spec/cli-options.md#fleet-delete` | 必須 | `runtime/spec/cli-options.md#fleet-delete` |
| L12 | `reference/cli.md:148` | `spec/cli-options.md#error-messages` | 必須 | `runtime/spec/cli-options.md#error-messages` |
| L13 | `reference/director.md:26` | `spec/cli-options.md#member-create` | 必須 | `runtime/spec/cli-options.md#member-create` |
| L14 | `reference/director.md:30` | `spec/cli-options.md#error-messages` | 必須 | `runtime/spec/cli-options.md#error-messages` |
| L15 | `reference/director.md:32` | `spec/cli-options.md#error-messages` | 必須 | `runtime/spec/cli-options.md#error-messages` |
| L16 | `reference/director.md:34` | `spec/cli-options.md#member-create` | 必須 | `runtime/spec/cli-options.md#member-create` |
| L17 | `reference/director.md:118` | `concepts/member-lifecycle.md` | 必須 | `runtime/concepts/member-lifecycle.md` |
| L18 | `reference/director.md:146` | `spec/cli-options.md#member-delete` | 必須 | `runtime/spec/cli-options.md#member-delete` |
| L19 | `reference/director.md:179` | `spec/cli-options.md#member-prompt` | 必須 | `runtime/spec/cli-options.md#member-prompt` |
| L20 | `reference/director.md:192` | `spec/multiplexer-backends.md#esc-safeguard` | 必須 | `runtime/spec/multiplexer-backends.md#esc-safeguard` |
| L21 | `reference/prompt-routing.md:48` | `spec/cli-options.md#member-prompt` | 必須 | `runtime/spec/cli-options.md#member-prompt` |
| L22 | `reference/recovery.md:38` | `concepts/monitoring.md` | 必須 | `runtime/concepts/monitoring.md` |
| L23 | `reference/supervision.md:15` | `spec/multiplexer-backends.md#push-notifications` | 必須 | `runtime/spec/multiplexer-backends.md#push-notifications` |
| L24 | `reference/supervision.md:122` | `spec/cli-options.md#member-create` | 必須 | `runtime/spec/cli-options.md#member-create` |
| L25 | `reference/coding-agent-overlays.md:28`（claudeのNote） | `concepts/monitoring.md`（inline code） | 必須 | `runtime/concepts/monitoring.md`（Markdown link化） |
| L26 | `reference/coding-agent-overlays.md:73`（codexのNote） | `concepts/monitoring.md`（inline code） | 必須 | `runtime/concepts/monitoring.md`（Markdown link化） |
| L27 | `reference/coding-agent-overlays.md:117`（opencodeのNote） | `concepts/monitoring.md`（inline code） | 必須 | `runtime/concepts/monitoring.md`（Markdown link化） |
| L28 | `reference/coding-agent-overlays.md:163`（TemplateのNote） | `concepts/monitoring.md`（inline code） | 必須 | `runtime/concepts/monitoring.md`（Markdown link化） |
| L29 | `docs/docs/spec/data-model.md:9` | `SPEC.md`（inline code） | 任意・人間向け再実装仕様 | `https://github.com/himkt/cafleet/blob/main/SPEC.md`（公開Markdown link） |

6つの直接参照先（`spec/cli-options.md`、`spec/multiplexer-backends.md`、`spec/data-model.md`、`spec/message-envelope.md`、`concepts/member-lifecycle.md`、`concepts/monitoring.md`）をseedとして、Markdownのローカルリンク先を推移的に同梱する。本文・見出し・anchorを保ったページ単位の生成物とし、リンク先のさらに先でもoffline参照を失わない。現時点の閉包は以下12ファイルで、すべて`docs/docs/<path>`から`skills/cafleet/reference/runtime/<path>`へ生成する。

| 同梱するpath | 同梱するpath |
|---|---|
| `spec/cli-options.md` | `spec/multiplexer-backends.md` |
| `spec/data-model.md` | `spec/message-envelope.md` |
| `spec/coding-agent-backends.md` | `spec/webui-api.md` |
| `concepts/member-lifecycle.md` | `concepts/monitoring.md` |
| `concepts/coding-agents.md` | `concepts/storage.md` |
| `how-to/mixed-backend-team.md` | `quickstart.md` |

正本の更新順はdocs→必要なSPEC同期→runtime生成→参照元skills更新とする。runtime生成はページ内の相対ディレクトリ構造を保ち、Markdown/画像等のローカル依存を含め、存在とanchorを検査する。生成物はリポジトリへ保存して既存のembedded skills対象に含める。checkoutからの手編集コピーや別の契約記述は維持しない。これらの参照はon-demandのままで、全12ページを各roleの起動時Required-readingに追加しない。

生成/check用manifestを`cafleet/tests/fixtures/runtime-reference-manifest.json`に置く。内容はL01–L29の参照元root（skills/docs）・path・heading path・元target/anchor・元表記形式（link/inline-code）・同一参照出現順・分類・置換先、および上記12生成pathの集合とする。行番号はidentityに使わない。Markdown parserでcode fenceを除く通常/reference形式リンクとローカル画像を検出し、移行後はこの置換先からmanifestとの対応を照合する。未分類の新しいskills外ローカル参照、既知参照の消失/重複数変化、生成閉包のファイル増減、生成物の内容差分はcheckを失敗させる。29件（必須28・任意1）という総数だけをassertせず集合・分類・出現数を比較する。L29はdocs正本と生成されたruntime/spec/data-model.mdの両方で期待URLを照合し、同じ参照の生成コピーを別IDとして二重計上しない。文書整理でリンクが減る場合も同じ変更でmanifestと設計対応表を更新してレビューし、黙って分類を落とさない。新しい必須参照は同じ同梱規則、任意参照へ変更する場合は人間向け補足である理由と具体的な公開URLをmanifestへ追加してレビューする。既存のdocs外参照L29は下記の公開リンク化を必要とする。既知の置換を反映した後に未分類のdocs外ローカル参照が残る場合は、推測でcheckoutへfallbackせず生成を失敗させる。

L25–L28は各backendおよびTemplateのload-bearing Note内のinline code `docs/docs/concepts/monitoring.md`を、ラベル「Monitoring」、href `runtime/concepts/monitoring.md`の実際のMarkdown linkへ変更する。4つのNoteそれぞれにリンクを残し、overlay自己完結性を保持する。既存のmonitoringページが同梱閉包にあるため、生成ページ数は12のままとする。

Markdownリンク検査に加えて、手書きskills本文・12ページのdocs正本・生成されたruntime本文のinline-code内にある静的な文書pathも抽出する。`docs/docs/`から始まるpathやrepoの`SPEC.md`などcheckoutにしか存在しない参照は、link化漏れとしてcheckを失敗させる。ローカル文書を読む指示のpathはinstalled-tree内で解決するか、manifestで明示した任意の公開リンクへ変換する。コード例のユーザー出力先・placeholder付きtask pathは参照と区別し、除外理由を検出ルールのfixtureへ記録する。L25–L28について元のinline codeへ戻すmutationを4箇所別々に与えて検査が失敗し、置換後は3backendとTemplateの計4headingから同梱ページを開けることをassertする。この検査によりMarkdown linkとして書かれていない必須参照も受入条件へ含める。

L29は`docs/docs/spec/data-model.md`冒頭を、次の意味の公開Markdownリンクへ更新してからruntimeを生成する: 「列単位の完全なDDL契約は再実装者向けの任意参照である[Repository specification](https://github.com/himkt/cafleet/blob/main/SPEC.md)に記載する」。SPECが権威ある再実装契約であることは変えない。インストール済みskillが担うfleet運用・通信・監視はCLIと同梱した契約ページで実行でき、CAFleet本体の再実装やDB schemaを手で操作するための完全DDLは実行時の必須読込ではないため、SPEC全体をoffline同梱しない。公開URLを開けなくてもこの運用手順は成立する。任意リンクはfixtureで期待URL・任意の表示・存在箇所を照合するが、offline検証中にnetwork取得を要求しない。

L29の変更で生成閉包は12ページのまま。docs正本と生成物のそれぞれにinline-code `SPEC.md`を再挿入したfixtureでは未変換参照として失敗させ、公開リンクへ変更した状態では成功させる。これをL25–L28の4つのmutationと合わせて検証する。inline-code検査の出力には元ページと生成先を明記し、生成物側を一律除外しない。presetのインストール先・配布元pathなど「文書を読む指示」ではない例は理由付きfixtureで区別し、権威先への参照L29を同じ除外に入れない。


生成は明示コマンドで実行して出力をレビューし、通常buildでは書き換えない。CIのcheckモードで生成結果との差分を検出する。二箇所に生成blockを置く場合も入力は同じにする。文言や見出しの全文一致テストを、生成block・CLI parser・wire出力・schema・リンク解決のテストへ置き換える。既存の重要な義務やエラー順の検証を削除で消さない。

supervisionのcaptureからactionへの判定をDirector側の正本とし、recoveryのidle>5分+unreadをstall根拠とする文章を除く。recoveryには死んだpane/接続断の確認、明示依頼されたshell dispatch、再spawn、monitor-first shutdownの差分だけ残す。capture失敗だけではpane消失を断定せず、doctorとpane一覧で確認する。

| 状態 | Director | monitor member |
|---|---|---|
| working | send/pingを延期 | 作業中として扱う |
| awaiting_user | paneのpromptを推測して回答せず、このroundのsendを延期。memberから明示的に届いた質問は通常のDirector relayへ。 | ユーザー待ちとして扱う |
| finished | 未完了の割当がある場合にfresh capture gateを通して再開、なければ正常な待機 | 既存roleのquiet判定条件に従う |
| stall_candidate | 連続するfacilitation turnの同一captureでquiet確認後に再開 | 連続wakeの同一SHAでquiet確認し、普通のmemberへquiet period当たり1回ping |
| unknown/死んだpane | pingせず原因調査とrecovery | Directorへevent報告 |

monitorがDirectorをpingする場合のみquiet確認に加えてunacked>0が必要という違いを保持する。Directorではunreadとmonitor eventは参考情報であり、単独でpingを許可しない。keystrokeを送った時点でcaptureは古くなり、次の送信にはfresh captureが必要。reply-soliciting messageへの即時回答と、memberが明示要求したshell dispatchの既存例外を保持する。backend固有cue・Codex managed-sessionの起動確認と再起動、one-shot単独呼出し、永続化後の通知失敗の再送禁止も保持する。

quickstartの主経路はsetup→環境確認→fleet bootstrap→member作成/通信→shutdownの短い一例に絞り、backend設定の詳細はリンクへ移す。「one-screen」のような実際の長さと合わない主張を除く。掲載するmonitor promptは実行可能なcanonical fixtureと同じ内容から生成し、次を欠かさない。

```text
ROLE DEFINITION: Open <そのbackendのインストール済みcafleet skill絶対path>/roles/monitor.md BEFORE any other action.
FLEET ID: {fleet_id}
DIRECTOR MEMBER ID: {director_member_id}
YOUR MEMBER ID: {member_id}
CODING AGENT: {coding_agent}
```

これは必要項目の形であり、そのままコピーする完成例ではない。実際の例はbackendの設定解決規則から絶対pathを埋め、roleが要求する`BASE`とload-bearing読込指示を含める。4つのidentity placeholderだけをspawn時に置換し、その他のliteral braceはescapeする。`fleet create --monitor-file`→ready/loop起動→起動ログ確認→`monitor live`→ordinary member許可の流れを説明・fixtureとも保持する。テストは3backendの実際のstage済みinstall pathを使ってfixtureを展開し、roleの存在と4識別値の欠落なしを確認する。

skillsのリンク検証は実installerで隔離HOME相当へ展開した2skillとpresetから開始し、manifestで固定した必須28参照（inline-codeからlink化する4件を含む）とその生成閉包のfile/anchorを辿り、任意L29の公開URLを照合して全29参照を検証する。docs正本と生成物を含む静的なinline-code参照の未変換も検出する。checkoutの絶対pathやfallbackを探索に使わず、外向き必須リンクは失敗させる。共有化後も各roleの必須読込表と各overlay節の自己完結性は保持し、リンク循環で読込責務が不明になる構造を作らない。

ユーザーの通常文の修正依頼はDirectorが意味を変えずuser-relay役のCOMMENT markerへ記録し、既存`ready (doc)`でDrafterへ渡す。曖昧な点だけ具体的に質問する。ユーザー自身によるmarker記入も引き続き受け付ける。メンバー間verb/pointer形式、marker解消、Approvedに未解決markerを残さない規則は維持する。

### 11. 検証とCI（F20）

各Stepで対象の回帰・契約テストを実行し、最後に全体ゲートを1回実行する。今回の設計作成中に実装テストを完了したとは記録しない。代表的なCLI JSONは完全bytesの小さなsnapshot、HTTPはwire shape/null/orderとstatus、内部純粋処理は意味のassertとし、長いprose全文snapshotを増やさない。

| ゲート | 完了条件 |
|---|---|
| Rust | `env CI=true mise //cafleet:test`、`mise //cafleet:lint`成功。既存480件に必要な回帰を追加し、整理時は旧assert→新検証の対応表でcoverage喪失を防ぐ。 |
| admin | 新規`mise //admin:test`、既存`mise //admin:lint`、`mise //admin:build`成功。fake timer/制御promiseで再現可能にする。 |
| docs | `mise //:docs-build`、生成block check、stage済みskillsのリンク/anchor/bootstrap fixture検証が成功。 |
| query | bounded履歴の取得行数、name lookup回数、lean rosterにactivity集計がないことをquery plan/traceで確認。固定msの性能閾値は使わない。 |
| 実画面 | 隔離DBでbroadcast ACK表示、初回障害/回復、fleet切替、200件省略表示をブラウザ確認。F04のアルゴリズムprobeとは別の実装完了証跡として記録。 |

CI lint jobにadmin lintを追加する。Rust taskの`//admin:build`依存を保持してfresh cloneのembedded dist生成を保証し、同じjobの明示admin build重複を除く。clippyが同一features/targetを検査していることを確認したうえでCIの追加cargo check相当を除き、手動typecheck taskは残す。別job間の必要なbuildまで重複扱いして削除しない。

miseの`--nocapture`例は現行toolchainでargv到達を検証して更新する。現在の`mise //cafleet:test my_test_name -- --nocapture`を成功例として再掲しない。実装時にtask側の明示的なtest/harness args forwardingを定義するか、実測したseparator配置を採用し、フィルタ1件とnocaptureが両方有効であることをテストfixtureで確認したコマンドだけ`.claude/rules/commands.md`へ掲載する。検証操作はfull-path mise taskを経由する。

---

## Implementation

> 完了したtaskはチェックと同じ編集で日時を記録する: `- [x] 作業 <!-- completed: 2026-09-05T14:30 -->`。以下36 taskはすべて未実装。各Stepは文書更新・対象回帰・実装・対象検証の順で進める。

### Step 1: pipeの詰まりとtimeoutを修正する

- [ ] §2に従いprocess契約文書を先に更新し、大量stdout/stderr・両stream・実timeout・非0終了の回帰fixtureを追加する。 <!-- completed: -->
- [ ] nonblocking drain・期限確認・直接子の回収を実装し、FD設定/IO失敗・子孫pipe保持を含めcleanupを検証する。 <!-- completed: -->

### Step 2: monitor登録のDB制約を追加する

- [ ] §3のschema/エラー/旧版による回復手順をdocs・SPECへ反映し、一意index migrationと事前診断を追加する。 <!-- completed: -->
- [ ] transaction内の重複判定と型付き競合エラーを導入し、2接続の決定的interleavingで敗者の副作用0を検証する。 <!-- completed: -->
- [ ] migrationのgrouped rollback、旧データ重複、再実行、behind-schemaとassets半分を考慮した回復fixtureを検証しchain guardを更新する。 <!-- completed: -->

### Step 3: pane作成の所有権と補償を統一する

- [ ] §4のエラー・cleanup契約を先に更新し、Herdr run失敗・placement Err/None・commit失敗のfixtureを追加する。 <!-- completed: -->
- [ ] backend/CLIのpane guardと登録補償を実装し、send_exitをkillへ変更して一次エラーとcleanup診断を検証する。 <!-- completed: -->
- [ ] fleet/memberの正常text/JSON出力の互換性、通知失敗契約、§4の失敗経路別補償順序と所有権移譲、二重cleanupなし、ID不明失敗の限界を検証する。 <!-- completed: -->

### Step 4: timelineの架空ACKを修正する

- [ ] §5のdelivery-only APIとrow capの説明をdocs・SPECへ反映し、summary除外のSQL/HTTP回帰を追加する。 <!-- completed: -->
- [ ] timeline filter、TS wire unionと純粋groupMessages、最小admin test taskを実装し、2配送/summary/ACK遷移と部分group表示を検証する。 <!-- completed: -->

### Step 5: broker型とadapter境界を整える

- [ ] §6の依存方向とwire維持を文書化し、member/placement/message/monitorを順にtyped rowへ移し、JSON再parse consumerを除く。 <!-- completed: -->
- [ ] 実process/notifierをruntimeへ移しwebui→cli依存を解消、必要なdomain errorだけ追加してCLI/HTTPへ写像する。 <!-- completed: -->
- [ ] JSON key order/null/text/終了コード/ガード順、MonitorRuntimeの各nullable fieldと0の意味、欠損nameの500、永続化済み通知失敗を契約テストで確認する。 <!-- completed: -->

### Step 6: 診断とqueryを簡素化する

- [ ] §6の診断・activity選択・fleet並び順・owner joinをdocs/SPECで確定し、型付き診断をguard/doctor/setupへ適用する。 <!-- completed: -->
- [ ] lean rosterとactivity queryを分け、name lookupを500 ID単位へ変更して空・重複・未知・deregisteredとquery回数を検証する。 <!-- completed: -->
- [ ] idleの0 clampだけを追加し、将来timestamp・全null・parse不能・ACK更新・summary送信を固定時計で検証する。 <!-- completed: -->

### Step 7: member履歴をWebUIで必要な量だけ取得する

- [ ] §5のlimit・validation順・省略時互換性をdocs/SPECへ追加し、broker optionsとHTTP→SQLのlimitを実装する。 <!-- completed: -->
- [ ] WebUIを201行取得・200行表示へ変更し、boundary/invalid/scope/順序と従来の無制限HTTP/CLIを検証する。 <!-- completed: -->

### Step 8: lifecycleとcaptureの共通処理を導入する

- [ ] §7を文書化しcreate optionsと共有spawn準備を導入、bootstrapの30秒共通deadlineと5秒補償budgetを実装・検証する。 <!-- completed: -->
- [ ] MonitorLeaseとsignal handle管理を実装し、登録・startup write/flush・tick・clear失敗とowner交代を注入して検証する。 <!-- completed: -->
- [ ] CaptureSnapshotとtext/JSON presenterを共通化し、ANSI/CR/Unicode/空の内容・時刻・hash契約を検証する。 <!-- completed: -->

### Step 9: fleet選択と非同期取得を整理する

- [ ] §8の取得責務とerror UXをdocsへ反映し、明示fleet client・URL authority・単一roster load ownerへ移行する。 <!-- completed: -->
- [ ] AbortController/世代/refresh coalesceとloading/success/error状態を導入し、再試行と更新失敗表示を実装する。 <!-- completed: -->
- [ ] 逆順応答・unmount・連続refresh・deep link・不正/削除fleet・送信先混在なしをadminテストで検証する。 <!-- completed: -->

### Step 10: assets交換を回復可能にする

- [ ] §9のinstall journal・ロック・失敗/中断回復契約をdocs/SPECへ反映し、filesystem/記録失敗fixtureを追加する。 <!-- completed: -->
- [ ] 同filesystem stage/backup交換、journal、DB記録、rollback/recoveryを実装しguard/doctorへ不完全状態判定を追加する。 <!-- completed: -->
- [ ] 同version・各swap/DB/rollback失敗・再起動・同時setup・symlink・別filesystem presetを隔離領域で検証する。 <!-- completed: -->

### Step 11: 文書とskillsの正本・参照を整理する

- [ ] §10に従い機械的CLI/schema blockの生成/checkを実装し、SPECの構造/詳細を維持して配布方式・formatterを含むdriftを修正する。 <!-- completed: -->
- [ ] supervision/recoveryの状態→行動を統一し、role差分・Required-reading・overlay自己完結性・174/175の契約を保持して反復を減らす。 <!-- completed: -->
- [ ] §10の29参照（必須28・任意1）と12ページ閉包をmanifestと明示生成/checkへ固定し、overlayの4参照をローカルlink化、data-modelのSPEC参照を任意の公開link化する。skillsへ同梱し、stage済みinstallのfile/anchor・公開URL照合、docs正本と生成物のinline-code未変換、未知/消失参照の検出を検証する。 <!-- completed: -->
- [ ] quickstartを短い主経路に編集し、3backendの完全bootstrap fixtureとready/live/identity/role参照を検証する。 <!-- completed: -->
- [ ] 通常文フィードバックをDirectorがCOMMENTへ変換するworkflowを更新し、role義務・marker解消・承認前条件を検証する。 <!-- completed: -->

### Step 12: テストとCIを整え全体を検証する

- [ ] docs_syncの各重要assertを実行可能契約/生成block/installed-tree検証へ対応づけ、prose固定だけの重複を削減する。 <!-- completed: -->
- [ ] full-path miseのtest/harness引数forwardingを実測してcommands規則を修正し、admin lint/testとbuild/typecheck重複整理をCIへ反映する。 <!-- completed: -->
- [ ] §11の全体ゲートと隔離DBのブラウザ確認を実行し、20件の対応・契約維持・エラー経路の証跡を記録する。 <!-- completed: -->
- [ ] first-class文書のdrift、未解決marker、task完了日時を最終確認し、実装成果のレビューを受ける。 <!-- completed: -->
