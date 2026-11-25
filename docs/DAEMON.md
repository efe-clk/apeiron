> **Türkçe:** [DAEMON.tr.md](DAEMON.tr.md)

# Apeiron Background Service

> **Status: Done.** This is the design of Stage 2 work item 3 ([ROADMAP.md](ROADMAP.md) §4): the background service, scheduled jobs and the restore test. The decision behind it is ADR-019 ([DECISIONS.md](DECISIONS.md)).

The background service (daemon) runs backups and checks on a schedule, without the user. Like the CLI it uses only the public API ([API.md](API.md)).

## 1. What it does

| Need | How | Status |
| --- | --- | --- |
| Back up without thinking about it | Jobs run on a schedule from a jobs file (section 3) | Done |
| A laptop that was off or asleep | A missed job runs once at the next chance (section 4) | Done |
| Copies elsewhere | A backup job can push to remotes after it succeeds | Done |
| Knowing that backups work | Scheduled `verify` and a restore test (section 5) | Done |
| Knowing when something failed | `apeiron daemon status` and a log (section 6) | Done |
| Watching it from a monitoring system | Metrics for Prometheus (section 7) and webhooks to a chat or an alerting service (section 8) | Done |

## 2. Commands

| Command | What it does | Status |
| --- | --- | --- |
| `apeiron daemon run` | Runs until stopped, starting each job when it is due | Done |
| `apeiron daemon once` | Runs every job that is due now, then exits; exit code 4 if one of them failed | Done |
| `apeiron daemon status` | Shows each job's last run, its result and the next run; exit code 4 if a job's last run failed | Done |
| `apeiron daemon metrics` | Prints the metrics of section 7, for instance for the text file collector of node_exporter | Done |
| `apeiron daemon test-webhooks` | Sends a test message to every webhook of section 8; exit code 4 if one was not accepted | Done |

All of them take `--jobs <file>` and `--state-dir <folder>` to use other files than the usual ones (sections 3 and 6). `apeiron restore-test [<snapshot>] [--sample-mb <n>]` runs the restore test of section 5 by hand.

`run` and `once` run the jobs at low priority, so that a backup gives way to what the user is doing: the CPU priority of a `nice` value of 10 and the lowest disk priority of the normal class on Linux, low CPU priority and throttled disk access on macOS, and background mode on Windows. `--priority normal` runs them as fast as other programs. Network speed is limited per job with `rate_kb` (section 3).

`daemon run` shows a desktop notification when a job fails after a run that did not, so a job retried every hour tells it once: with `notify-send` on Linux, `osascript` on macOS and a PowerShell balloon on Windows. The job's name and message are passed as arguments, never inside a script. A computer without the tool, such as a server, shows nothing; the log and `daemon status` still say what happened. `--no-notify` turns it off.

The operating system starts `apeiron daemon run` at login: a systemd user unit on Linux, a launchd agent on macOS and a Task Scheduler task on Windows. The user guide gives the file for each ([GUIDE.md](GUIDE.md) §8); no administrator rights are needed. Only one daemon runs per user; a second one stops with a "locked" error.

## 3. The jobs file

The file is `jobs.toml` in the user's configuration folder: `~/.config/apeiron` on Linux, `~/Library/Application Support/apeiron` on macOS and `%APPDATA%\apeiron` on Windows. `--jobs <file>` uses another file.

```toml
[job.documents]
repo = "/run/media/user/disk/apeiron"
password_file = "~/.config/apeiron/password"   # only for an encrypted repository
do = "backup"
folder = "~/Documents"
ref = "backup/documents"
every = "day"
at = "02:30"
push = ["nas"]                                 # remotes to push to afterwards

[job.check]
repo = "/run/media/user/disk/apeiron"
password_file = "~/.config/apeiron/password"
do = "verify"
every = "week"

[job.restore-test]
repo = "/run/media/user/disk/apeiron"
password_file = "~/.config/apeiron/password"
do = "restore-test"
ref = "backup/documents"
every = "week"
```

| Key | Meaning |
| --- | --- |
| `repo` | The repository the job works on; a folder remote can be named here directly to test that copy |
| `password_file` | The first line is the password or recovery key of an encrypted repository |
| `do` | `backup`, `push`, `pull`, `verify`, `restore-test`, `gc`, `prune` ([FORMAT.md](FORMAT.md) §13.4), `repair` ([FORMAT.md](FORMAT.md) §8.3) or `sync` ([SYNC.md](SYNC.md)) |
| `folder`, `ref`, `message` | For `backup`: what to store and which ref to move; for `sync`: the folder and the ref, by default `sync/<folder name>` |
| `remote`, `mode` | For `sync`: the remote, and `two-way` (default), `send` or `receive`; for `pull` and `repair`: the remote |
| `ref = "*"` | For `push` and `pull`: every ref, to keep a replica or a standby ([REPLICATION.md](REPLICATION.md) §3) |
| `push` | For `backup`: remotes to push the ref to after a successful backup; for `push`: the remotes |
| `keep_last`, `keep_daily`, `keep_weekly`, `keep_monthly`, `keep_yearly` | For `prune`: what to keep of `ref`; `push` then names the remotes that follow |
| `exclude` | For `backup` and `sync`: a list of patterns to leave out ([CLI.md](CLI.md) §4.1, [SYNC.md](SYNC.md) §5.1) |
| `every` | `hour`, `day`, `week`, or a number of minutes, hours or days: `15m`, `6h`, `2d` |
| `at` | For `day` and `week`: the earliest local time of day, `HH:MM` |
| `sample_mb` | For `restore-test`: how much to restore, default 100 |
| `on_change` | For `backup` and `sync`: also run after a change to the folder, once it has been quiet this long: a number with `s`, `m` or `h`, from `5s` to `24h` (section 4) |
| `source`, `pause` | For `backup`: an application-consistent source instead of `folder`, such as `postgres:shop`, and `true` to pause a docker source's containers; not in a tenant's jobs ([CONSISTENT.md](CONSISTENT.md)) |
| `alarm_percent`, `hold_on_alarm` | For `backup`: the ransomware alarm's percentage (50 by default, 0 turns it off), and `true` to place a legal hold on the ref when it goes off; a run with the alarm fails and pushes nothing ([CLI.md](CLI.md) §4.3) |
| `rate_kb` | For `backup`, `push`, `pull`, `repair` and `sync`: the most kibibytes per second moved to or from remotes, on average ([SYNC.md](SYNC.md) §2); no limit by default |

`~` at the start of a path is the home folder. A job name follows the ref name rules without `/`. An invalid file stops `daemon run` and `daemon once` with an error that names the line; the daemon reads the file again when it changes and keeps the old jobs if the new file is invalid. The Schedule page of the desktop interface shows the jobs and edits this file, and saves only a valid one (DESKTOP.md §2).

### 3.1 Jobs of a tenant

Each tenant of a web server (TENANTS.md) can have jobs of its own, which reach only its repository and its folders. They are in `<configuration folder>/tenants/<name>/jobs.toml`, with their state and log in `<state folder>/tenants/<name>/daemon`, and `apeiron daemon run --tenant <name>` runs them; `once` and `status` take `--tenant` too. Whoever runs the server starts one such daemon per tenant, for example with a systemd unit `apeiron-daemon@.service` whose instance name is the tenant's.

A tenant's jobs file has no `repo` and no `password_file`: every job works on the tenant's repository, opened with the tenant's `password_file` (TENANTS.md §2). The `folder` of a `backup` or `sync` job must be a full path inside one of the tenant's `folders`, without `..`; other jobs are as in section 3, and `sync` keeps a tenant's folder in step with a remote of its repository. A file that breaks these rules is refused as invalid.

An `admin` of the tenant edits its jobs file on the Schedule page of the web interface, which shows each job's last and next run as on the desktop and saves only a valid file. Each save is recorded as `schedule_save` in the tenant's audit log (AUDIT.md §1), with the names of the jobs.

### 3.2 Jobs of a web server

`apeiron web --schedule` runs jobs inside the web server of one repository, so an `admin` sets them up and watches them from any computer, on the Schedule page. The rules are those of a tenant (section 3.1): every job works on the served repository, opened with its `--password-file`, and a `folder` must be inside a `--folder` of the server. The jobs file is `<configuration folder>/web-jobs.toml`, with the state and log in `<state folder>/web-daemon`.

The jobs run on a thread of the server, at its priority, and stop when it stops. A second server, or an `apeiron daemon run` with the same state folder, cannot start while one runs them: the start fails with a "locked" error. Each save is recorded as `schedule_save` in the audit log.

## 4. When a job runs

```mermaid
flowchart LR
    W["Wake up<br/>(next due job, at most 1 minute)"] --> D{"Job due?"}
    D -- no --> W
    D -- yes --> R["Run it"]
    R --> S["Write the result<br/>to the state file"]
    S --> W
```

- A job is due when `every` has passed since its last start. With `at`, it waits until that time of day on the day it becomes due.
- A `backup` or `sync` job with `on_change` also runs when its folder changes: once a change is followed by that long without another one, so a burst of saves gives one run. Changes inside the repository, when it lies in the folder, do not count. Only `daemon run` watches folders, from when it starts; a folder that cannot be watched is noted in the log and the job keeps its schedule. Linux limits how many folders one user can watch (`fs.inotify.max_user_watches`); a very large tree may need a higher limit.
- A run that was missed because the computer was off or asleep is made once at the next chance, not once for every missed period.
- Jobs run one at a time, in the order of their names. A job that is still running when another becomes due is not interrupted.
- A job that fails is tried again after one hour, until it succeeds or its next regular run comes. A job that meets a "locked" repository (for example during garbage collection run by hand) is tried again after 10 minutes.
- Stopping the daemon cancels the running job; the repository stays in its previous state (CLI.md §1).

## 5. Restore test

A backup is proven only by restoring it (VISION.md). The restore test does this without touching the user's files:

1. Take the snapshot the ref points to.
2. Choose files from it at random until `sample_mb` is reached; at least one file.
3. Restore them into a new temporary folder, which reads every chunk and checks its hash.
4. Split each restored file into chunks again and check that the chunk ids and the size equal those in the snapshot. This catches errors in the restore path itself, not only damaged data.
5. Delete the temporary folder and record the result: files and bytes checked, or what was wrong.

## 6. State and log

| File | Folder | Content |
| --- | --- | --- |
| `state.json` | `~/.local/state/apeiron` on Linux, the configuration folder on macOS, `%LOCALAPPDATA%\apeiron` on Windows | Per job: last start and end, result (`ok`, `warnings`, `failed` or `locked`), message and the end of the last run that succeeded; `status` works out the next run from it. Replaced atomically after every run |
| `daemon.log` | The same folder | One line per job run and per error; at 1 MiB it is renamed to `daemon.log.1` and a new one starts |

## 7. Metrics

`apeiron daemon run --metrics <address>` answers Prometheus at `http://<address>/metrics`, such as `--metrics 127.0.0.1:9184`; `apeiron daemon metrics` prints the same text once. The values are worked out from the state file whenever they are asked for. The address has no password, so it belongs on this computer or a network only the monitoring system reaches; the metrics show job names and times, never messages or file names.

| Metric | Labels | Value |
| --- | --- | --- |
| `apeiron_build_info` | `version` | 1 |
| `apeiron_jobs` | | Jobs in the jobs file |
| `apeiron_job_last_start_timestamp_seconds`, `apeiron_job_last_end_timestamp_seconds` | `job`, `do` | When the last run started and ended, in Unix seconds |
| `apeiron_job_last_duration_seconds` | `job`, `do` | How long the last run took |
| `apeiron_job_last_success_timestamp_seconds` | `job`, `do` | When the last run that succeeded ended; missing before the first |
| `apeiron_job_last_result` | `job`, `do`, `result` | 1, with the result of the last run: `ok`, `warnings`, `failed` or `locked` |
| `apeiron_job_failing` | `job`, `do` | 1 when the last run failed, otherwise 0 |
| `apeiron_job_next_run_timestamp_seconds` | `job`, `do` | When the job runs next |

A web server gives the same metrics of its scheduled jobs, with those of its repository, to an `admin` ([WEB.md](WEB.md) §4). A rule that warns when a backup has not succeeded for two days:

```yaml
- alert: ApeironBackupOld
  expr: time() - apeiron_job_last_success_timestamp_seconds{do="backup"} > 2 * 86400
```

## 8. Webhooks

A webhook tells a chat or an alerting service when a job starts failing, succeeds again or ends. Webhooks are in their own file beside the jobs file: `jobs.webhooks.toml` beside `jobs.toml`, `web-jobs.webhooks.toml` beside a web server's jobs and `tenants/<name>/jobs.webhooks.toml` for a tenant's. Only whoever runs the service writes it; the Schedule page cannot, so nobody signed in to a web server can make it call an address. The file is read for each run, so a change counts at once.

```toml
[webhook.team-chat]
url_file = "~/.config/apeiron/slack-url"   # or url = "https://..."
format = "slack"

[webhook.on-call]
format = "pagerduty"
routing_key_file = "~/.config/apeiron/pagerduty-key"
jobs = ["documents"]
```

| Key | Meaning |
| --- | --- |
| `url`, `url_file` | The address to post to, or a file whose first line it is; a chat's address is a secret, so `url_file` keeps it out of this file. PagerDuty has its own by default |
| `format` | `json` (default), `slack`, `teams` (a Workflows webhook of Microsoft Teams) or `pagerduty` (Events API v2) |
| `on` | `failed`: a run fails after one that did not; `recovered`: a run succeeds after one that failed; `done`: every run. Default `["failed", "recovered"]`; PagerDuty takes only these two, opening and closing one alert per job and computer |
| `jobs` | Only these jobs; every job by default |
| `routing_key_file` | For PagerDuty: a file whose first line is the integration's routing key |

The `json` format posts `{"event", "job", "do", "result", "message", "start", "end", "host"}`; the others a line such as `Apeiron on nas: job documents failed: <message>`. A webhook that cannot be reached, or answers with an error, is noted in the log after at most 10 seconds and never fails the job. An invalid file is noted in the log the same way; `apeiron daemon test-webhooks` shows what is wrong with it.

## 9. Traces

With `OTEL_EXPORTER_OTLP_ENDPOINT` set, such as `http://localhost:4318`, the background service sends an OpenTelemetry span for each run of a job, and a web server ([WEB.md](WEB.md)) one for each operation it is asked, to that collector as OTLP over HTTP with JSON; Jaeger, Grafana Tempo, Honeycomb, Datadog and the OpenTelemetry Collector take it. Without it, nothing is sent.

| Variable | Meaning |
| --- | --- |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | The collector; spans go to `<it>/v1/traces` |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | The full address for spans instead, used as it is |
| `OTEL_EXPORTER_OTLP_HEADERS` | `key=value` pairs separated by commas, such as a service's API key |
| `OTEL_SERVICE_NAME` | `service.name` of the spans; `apeiron` by default |

A job's span is named `<do> <job>`, such as `backup documents`, with the attributes `apeiron.job`, `apeiron.action`, `apeiron.result` and `apeiron.repository`; a failed run has the status error and its message. An operation's span is named after the operation, such as `refs`, with `apeiron.operation` and, for a signed-in user, `enduser.id`. Spans are sent in batches from a thread of their own, after two quiet seconds or 100 spans, so a slow collector never holds up a job or a request; one that cannot be reached is noted on standard error. Each span is a trace of its own; spans inside a job, such as reading files and writing packs, are not sent yet.

## 10. Not part of this work item

| Item | Where |
| --- | --- |
| Jobs of a web server for several tenants inside the server | Each tenant's jobs run in an `apeiron daemon run --tenant` of their own (section 3.1) |
| Spans inside a job, and trace context passed in from a caller | Later |
| OTLP over gRPC or protobuf | Not planned; collectors take JSON over HTTP |

## 11. Crates and libraries

| Crate | Role | Status |
| --- | --- | --- |
| `apeiron-daemon` | Jobs file, schedule, state, restore test, the run loop and watching folders; uses the public `apeiron` crate, and `apeiron-sys` to lower its priority | Done |
| `apeiron-cli` | The `apeiron daemon` commands | Done |

| Need | Proposed library | Reason |
| --- | --- | --- |
| Local time of day and time zones | `jiff` | Correct across daylight saving changes on Linux, macOS and Windows; pure Rust |
| Configuration and state folders | `dirs` | Small; knows the right folder on each system |
| Jobs file and state | `toml`, `serde_json` | Already used |
| Webhooks and traces | `reqwest` (blocking), `gethostname` | Already used by the remotes and the CLI; the blocking client fits the daemon's thread |
| Watching folders for `on_change` | `notify` | The common Rust watcher: inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows; it runs its own thread and needs no Tokio |

The daemon needs no Tokio: one thread sleeps until the next job is due and wakes at least every minute to notice a changed jobs file or a clock that jumped.
