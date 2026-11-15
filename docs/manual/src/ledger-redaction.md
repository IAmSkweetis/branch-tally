# Ledger Redaction

A committed [ledger](ledger.md) is part of your repository. If the repository is public, so is
every line of `.tally/ledger.jsonl`, and so is every line it ever held, because git keeps them.
This page says exactly which fields a reader can see, what `record` hashes by default, and what
hashing does not protect against.

## What a committed ledger makes public

`record` hashes identifiers by default. Each line then carries a `redaction` field, and the
identifiers in it are replaced by a keyed hash: 32 lowercase hex characters. Everything else is
written as it is.

| Field | Redaction off | Redaction on (the default) | On, with names hashed too |
| ----- | ------------- | -------------------------- | ------------------------- |
| `request_id` | readable | hashed | hashed |
| `session_id` | readable | hashed | hashed |
| `agent_id` | readable | hashed | hashed |
| `subagent`: its `agent_id` | readable | hashed | hashed |
| `subagent`: its `subagent` label, when that label is an agent id | readable | hashed | hashed |
| `subagent`: its `subagent` label, when that label is an agent name | readable | readable | hashed |
| `subagent`: its `workspace` worktree name | readable | hashed | hashed |
| `origin`: the worktree name | readable | hashed | hashed |
| `agent`, `skill` | readable | readable | hashed |
| `model` | readable | readable | readable |
| `recorded_for`, `branch`: an agent branch, `worktree-agent-<agentId>` | readable | `worktree-agent-` then the hashed id | `worktree-agent-` then the hashed id |
| `recorded_for`, `branch`: any other branch name | readable | readable | readable |
| `mode`, `basis`, `kept_by` | readable | readable | readable |
| `usage` (token counts) | readable | readable | readable |
| `ts` (the call's time, full precision) | readable | readable | readable |
| `recorded_at` (when the line was written) | readable | readable | readable |
| `v`, `harness` | readable | readable | readable |
| `redaction` | absent | the salt's fingerprint | the salt's fingerprint |

"Readable" means the value is in the file as it is. The `subagent` label is the subagent's agent
name when Claude Code recorded one, and its agent id otherwise. An id is always hashed. Worktree
names are hashed with the ids because Claude Code names a subagent's worktree `agent-<agentId>`,
so the name would give away the id. For the same reason, a branch named `worktree-agent-<agentId>`,
which Claude Code gives an agent's worktree and which a call it can't link to a dispatcher keeps as
its branch, is written as `worktree-agent-` followed by the hashed id. Every other branch name is
written as it is. `origin` values of `"main"`, and a `workspace` of `"main"` or `"unknown"`, are
not names and stay as they are.

The `redaction` field is a fingerprint of the salt the line was hashed with: the first 16 hex
characters of a SHA-256 digest of it. It tells a reader that the line is redacted, and which salt
was used. It does not give away the salt, which is in the repository anyway (see
[The salt](#the-salt)). When redacting, the field alone is not believed: a line is left alone only
when its request, session and agent ids are also hashes. A line that carries the field over raw
ids, through a bug or a hand edit, is hashed again by
[`ledger redact`](#rewriting-older-lines) and counted as `restamped`. The line counts in the
[mixed-ledger](#mixed-ledgers) note go by the field alone.

A report asked for an agent branch by its real name, such as `worktree-agent-a1`, hashes the name
with the local salt and finds the ledger's lines under both forms. Without a readable local salt it
finds only lines that hold the name as it is.

Hashing changes no number. Models, token counts and times are kept, and branches are either kept
or hashed the same way every time, so a report from a redacted ledger has the same totals as one
from an unredacted ledger of the same calls.

Here is a line written by `record` with the defaults, from a synthetic test repository. It is
shown across several lines; in the file it is one line.

```json
{
  "v": 1,
  "request_id": "9dc50c5d56142e117667bf83d9431588",
  "harness": "claude-code",
  "session_id": "69a9385f728887a9d8cb488b91ed71c3",
  "agent_id": "814abc4a50c5d0f9c5b6d2c0a01c0a10",
  "ts": "2026-01-01T00:00:00.000Z",
  "model": "claude-test-1",
  "recorded_for": "feat/x",
  "mode": "claim",
  "branch": "feat/x",
  "basis": "dispatcher-rolled-up",
  "kept_by": "exact",
  "subagent": {
    "workspace": "unknown",
    "subagent": "814abc4a50c5d0f9c5b6d2c0a01c0a10",
    "agent_id": "814abc4a50c5d0f9c5b6d2c0a01c0a10"
  },
  "origin": "main",
  "usage": {"input": 10, "cache_write_5m": 0, "cache_write_1h": 0, "cache_read": 30, "output": 20, "thinking": null},
  "recorded_at": "2026-11-15T02:20:07Z",
  "redaction": "b4aba62c45a92477"
}
```

### Never written

Whatever the settings, the ledger never holds message content (prompts, responses, tool input or
output, code), the working directory, or any path. See
[What is stored](ledger.md#what-is-stored).

## What hashing does not hide

Read this before you rely on redaction in a public repository.

- **Git history keeps every line ever committed.** Turning redaction on, or running
  [`ledger redact`](#rewriting-older-lines), changes the file from then on. Earlier commits still
  hold the raw ids, and anyone with a clone can read them with `git log -p`. Removing them means
  rewriting the repository's history, which branch-tally does not do.
- **The salt is committed, so it is not a secret.** Every clone has to produce the same hashes, or
  lines recorded on two machines would never match. So anyone who already holds an id, such as a
  request id or session id from their own transcript, can hash it with the committed salt and see
  whether it is in your ledger. Redaction stops a reader from reading ids off the repository and
  from linking them with the same ids elsewhere, such as in another repository's ledger. It does
  not protect anything from someone who holds the original transcript.
- **A guessable value is not hidden.** Ids are random, so they can't be guessed. Agent and skill
  names often can: with `ledger-redact-names` on, anyone can hash a list of likely names with the
  committed salt and compare.
- **The shape stays visible.** The same id always hashes to the same value, so every line of one
  session carries the same hashed `session_id`. A reader can still count sessions and subagents,
  see which calls belong together, and see when each call was made, to the second.
- **Branch names, models and times are not hashed.** Only an agent branch's id is. See the table
  above.
- **Hook captures are not hashed.** `.tally/captures.jsonl` still holds real session ids, agent
  ids and branch names, because `record` needs them to attribute calls. If you commit it, they are
  public. See [Hook captures](captures.md#what-it-never-holds).

## The salt

The hash is HMAC-SHA256 keyed with a per-repository salt: 32 random bytes, stored as 64 lowercase
hex characters in a file named `salt` in the **same `.tally/` directory as the ledger it hashes**.

- **Where it is.** For the default ledger that is `.tally/salt` at the root of the checkout `record`
  runs in. A linked git worktree has its own `.tally/salt`, committed on the worktree's branch with
  its ledger. For an explicit `--ledger <path>`, it is `.tally/salt` in the main checkout.
- **How it is created.** When that `.tally/` has no salt, `record` with redaction on copies the main
  checkout's `.tally/salt` if there is one, so every worktree on the machine hashes alike, and
  otherwise creates a new one. `ledger redact` does the same. A `record` creates the salt even
  when it appends no lines. Nothing is created in a checkout that has not
  [opted in](ledger.md#opt-the-repository-in): with no `.tally/`, `record` writes nothing at all,
  and with redaction on it never writes raw ids.
- **Commit it.** Every clone must hash with the same salt, so the salt belongs in the repository
  next to the ledger. A clone without it creates its own, and its lines then never match yours.
- **Create it once, before anyone else records.** If two machines each create a salt before either
  is committed, git reports a conflict on `.tally/salt` when the branches meet. Keeping one side
  resolves the conflict, but the lines hashed under the other salt keep their fingerprint, and the
  ledger is then [mixed](#mixed-ledgers). To avoid that, create the salt as part of opting in:

  ```sh
  mkdir .tally
  touch .tally/.gitkeep
  branch-tally ledger redact     # creates .tally/salt; there is nothing to rewrite yet
  git add .tally/.gitkeep .tally/salt
  git commit -m "chore: opt in to branch-tally"
  ```

  `ledger redact` prints
  `redacted 0 line(s), 0 restamped, 0 already redacted, 0 unparseable left as-is`.
- **A damaged salt stops the command.** A `.tally/salt` that is not 64 lowercase hex characters is
  never replaced automatically, because replacing it would orphan every line already hashed under
  it. `record` and `ledger redact` fail with exit 1:

  ```text
  <path> is empty or damaged: it is not a salt (64 lowercase hex characters). It is never replaced automatically. It can be deleted to have a new one created; lines already hashed with the old salt will then read as a different salt
  ```

  If the salt is committed, restore the committed copy from git instead. Deleting the file gets a
  new salt, and the ledger then holds lines under two salts: see [Mixed ledgers](#mixed-ledgers).
- **The guard against links applies.** Like the other files in `.tally/`, the salt is not read or
  created through a symbolic link. See
  [Symbolic links are refused](ledger.md#symbolic-links-are-refused).

[`ledger check`](cli.md#branch-tally-ledger-check) does not look at `.tally/salt`. A pull request
that changes it passes the check, so review changes to it by hand.

## Turning it off, or hashing names too

`record` has two settings of its own, separate from the report's `redact` and `redact-names`:

| Config key | Default | Flags | Does |
| ---------- | ------- | ----- | ---- |
| `ledger-redact` | `true` | `--ledger-redact`, `--no-ledger-redact` | Hash ids, worktree names and the ids in agent branch names in the lines `record` writes. |
| `ledger-redact-names` | `false` | `--ledger-redact-names`, `--no-ledger-redact-names` | With `ledger-redact` on, also hash agent and skill names. |

A flag overrides the [config file](config.md) for one run:

```sh
branch-tally record --no-ledger-redact        # write raw ids this time
branch-tally record --ledger-redact-names     # hash agent and skill names too
```

Use the same `ledger-redact-names` setting on every machine that records into one ledger. For a call
made by a named agent or skill, a line with the names hashed and a line without have the same key
and different contents, which is a [conflicting ledger line](ledger.md#conflicting-ledger-lines).

### An explicit `--ledger` needs `.tally/`

For an explicit `--ledger <path>`, the salt lives in the main checkout's `.tally/`. So with
redaction on, `record --ledger <path>` in a repository whose main checkout has no `.tally/`
directory stops with exit 1, and writes nothing:

```text
ledger redaction needs a .tally/ directory in the main checkout to hold its salt: create it, or pass --no-ledger-redact
```

Create `.tally/` in the main checkout, or pass `--no-ledger-redact` to write raw ids to that file.

### Worktrees

A worktree whose checkout has `.tally/` records into its own ledger with its own salt, whether or
not the main checkout has opted in. When the main checkout's salt was copied in, the worktree's
`.tally/salt` is identical to it, so merging the worktree's branch back leaves nothing to resolve.
When the main checkout had no salt, the worktree creates its own. If the main checkout's branch
later gets a different one, git reports a conflict on `.tally/salt` when the branches meet, as
described above.

## Rewriting older lines

Lines written before redaction was on, or written with `--no-ledger-redact`, keep their raw ids.
`branch-tally ledger redact` rewrites them:

```sh
branch-tally ledger redact
```

```text
redacted 0 line(s), 1 restamped, 3 already redacted, 1 unparseable left as-is
note: 1 unparseable line(s) were left as-is and may still hold raw ids
```

- It hashes every line that parses and has no `redaction` field, with the salt beside the ledger,
  copying or creating it as described in [The salt](#the-salt). A rewritten line then matches the
  line `record` writes for the same call, so the two collapse into one in a report.
- A line that carries a `redaction` field over raw ids is hashed too, and counted as `restamped`.
- Lines that are genuinely redacted are left byte for byte as they are, whichever salt they were
  hashed with. Lines it can't parse are counted and left as they are, raw ids included, and a note
  on stderr says so. That includes a line whose `ts` is not strict RFC 3339 (see
  [Timestamps](cli.md#timestamps)).
- `--ledger-redact-names` applies only to the lines it rewrites. Lines already redacted without
  hashed names keep their readable names. With the option on and redacted lines present, a note
  on stderr says the names opt-in can't be backfilled onto them. Both notes are listed in the
  [CLI reference](cli.md#branch-tally-ledger-redact).
- It runs even when `ledger-redact` is off in your config, because you asked for it. It uses
  `ledger-redact-names` from the config file, or `--ledger-redact-names` /
  `--no-ledger-redact-names`.
- It writes the new ledger to a temporary file next to it and replaces the ledger only when that
  succeeds. If the ledger changed in the meantime, for example because a `record` appended to it,
  it replaces nothing and fails with exit 1:
  `the ledger changed while it was being rewritten; nothing was replaced, run it again`. It does
  not commit anything.
- Without a `.tally/` directory beside the default ledger, it changes nothing and prints
  `no .tally/ directory: nothing redacted (create it to opt in)`. With `--ledger <path>` and no
  `.tally/` in the main checkout, it fails with exit 1:
  `ledger redact needs a .tally/ directory in the main checkout to hold its salt: create it first`.

It is the one command that changes lines already in the ledger, so the next commit removes the old
lines and adds rewritten ones. Two consequences:

- **History still has the raw lines.** See
  [What hashing does not hide](#what-hashing-does-not-hide).
- **`ledger check` fails a pull request that rewrites base lines,** with
  `removed base line: a base line was removed or changed`, because the base's lines may only be
  appended to. Treat it like the repair described under
  [Conflicting ledger lines](ledger.md#conflicting-ledger-lines): open a pull request that only
  runs `ledger redact`, say why in its description, and merge it with the bypass your repository's
  rules allow.

The options are listed in the [CLI reference](cli.md#branch-tally-ledger-redact).

## Mixed ledgers

A ledger can hold unredacted lines next to redacted ones, for example after redaction was turned
on, or lines hashed under two different salts. Such a ledger is read normally, never refused. But
a call recorded once in each form appears under two different keys, so it is counted twice. That
makes the total an overcount, not a floor. `report --from-ledger` and `project` say so on stderr:

```text
note: ledger holds 3 unredacted line(s) and 3 redacted line(s) under 1 salt(s); a call recorded in more than one form may be counted twice. Run `branch-tally ledger redact` to rewrite the unredacted lines
```

When the ledger has no unredacted lines, only lines under more than one salt, the note ends after
"counted twice": `ledger redact` can't fix that, because the raw ids behind a hash are gone.

`report` also puts the warning in the report itself, so it reaches a pull request body written with
`--pr`. The Markdown report adds a statement after the session table, on one line:

```text
**Total may count some calls twice**: the ledger mixes 2 unredacted and 2 redacted line(s) under 1 salt(s), so a call recorded in both forms is counted twice; run `branch-tally ledger redact` to collapse them. This is an overcount, not a floor.
```

The JSON report has a top-level `doubleCount` object:

```json
"doubleCount": { "mixedLedger": { "unredactedLines": 2, "redactedLines": 2, "distinctSalts": 1 } },
```

`project` prints only the stderr note. Its report and chart carry no statement.

### Combining the ledger with live transcripts

`report --from-ledger --with-live` hashes this machine's live calls with the local salt before
combining them with the ledger, so a call in both is counted once. The local salt is the one in the
ledger's own `.tally/`, or else the main checkout's. When it can't do that, it says why on stderr,
and the total and the repository-wide counts may count a call twice:

```text
note: live calls could not be matched against the redacted ledger lines (<why>); the total and the repository-wide counts may count a call twice
```

| `<why>` | Meaning |
| ------- | ------- |
| `no salt in .tally/` | The ledger has redacted lines, but neither the ledger's `.tally/` nor the main checkout's has a salt. Pull the commit that added it. |
| `the salt is unreadable` | `.tally/salt` exists but is not a valid salt. |
| `the local salt matches none of the redacted lines` | The redacted lines were hashed with a different salt than the one here. |
| `the local salt matches only some of the redacted lines` | The ledger holds lines under more than one salt, and only some are under the local one. |

The report itself carries the same warning. In Markdown the bold statement reads, for example:

```text
**Total may count some calls twice**: live calls could not be matched to redacted ledger lines (the local salt matches none of the redacted lines), so a call in both may be counted twice. This is an overcount, not a floor.
```

In JSON, `doubleCount` holds `liveMatch`: `"no-salt"`, `"salt-unreadable"`,
`"fingerprint-mismatch"` or `"partially-matched"`. When the ledger is also mixed, the statement
gives both causes and `doubleCount` holds both keys.

## Report redaction is separate

Everything above is about the ledger file. What a report prints is set by `report`'s own
`--redact`, which is still **off** by default, including for `report --pr`. Without it, a report,
and the pull request body it writes, shows:

- the first eight characters of each session id (from a redacted ledger, of the hashed id);
- agent and skill names;
- worktree names and subagent labels;
- each session's working-time span, to the minute.

`--redact` replaces these with generic labels and dates. See [Redaction](cli.md#redaction).
