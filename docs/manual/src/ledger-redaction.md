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
| `recorded_for`, `branch` | readable | readable | readable |
| `mode`, `basis`, `kept_by` | readable | readable | readable |
| `usage` (token counts) | readable | readable | readable |
| `ts` (the call's time, full precision) | readable | readable | readable |
| `recorded_at` (when the line was written) | readable | readable | readable |
| `v`, `harness` | readable | readable | readable |
| `redaction` | absent | the salt's fingerprint | the salt's fingerprint |

"Readable" means the value is in the file as it is. The `subagent` label is the subagent's agent
name when Claude Code recorded one, and its agent id otherwise. An id is always hashed. Worktree
names are hashed with the ids because Claude Code names a subagent's worktree `agent-<agentId>`,
so the name would give away the id. `origin` values of `"main"`, and a `workspace` of `"main"` or
`"unknown"`, are not names and stay as they are.

The `redaction` field is a fingerprint of the salt the line was hashed with: the first 16 hex
characters of a SHA-256 digest of it. It tells a reader that the line is redacted, and which salt
was used. It does not give away the salt, which is in the repository anyway (see
[The salt](#the-salt)).

Hashing changes no number. Models, branches, token counts and times are kept, so a report from a
redacted ledger has the same totals as one from an unredacted ledger of the same calls.

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
- **Branch names, models and times are not hashed at all.** See the table above.
- **Hook captures are not hashed.** `.tally/captures.jsonl` still holds real session ids, agent
  ids and branch names, because `record` needs them to attribute calls. If you commit it, they are
  public. See [Hook captures](captures.md#what-it-never-holds).

## The salt

The hash is HMAC-SHA256 keyed with a per-repository salt: 32 random bytes, stored as 64 lowercase
hex characters in `.tally/salt` in the **main checkout**. Every git worktree on the machine uses
that one file.

- **It is created for you.** The first `record` with redaction on, or the first `ledger redact`,
  creates it when it is missing, in a repository that has
  [opted in](ledger.md#opt-the-repository-in). Nothing is created where `.tally/` does not exist.
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

  `ledger redact` prints `redacted 0 line(s), 0 already redacted, 0 unparseable left as-is`.
- **A damaged salt stops the command.** A `.tally/salt` that is not 64 lowercase hex characters is
  never replaced, because replacing it would orphan every line already hashed under it. `record`
  and `ledger redact` fail with exit 1:

  ```text
  <path> is not a valid salt (expected 64 lowercase hex characters)
  ```

- **The guard against links applies.** Like the other files in `.tally/`, the salt is not read or
  created through a symbolic link. See
  [Symbolic links are refused](ledger.md#symbolic-links-are-refused).

[`ledger check`](cli.md#branch-tally-ledger-check) does not look at `.tally/salt`. A pull request
that changes it passes the check, so review changes to it by hand.

## Turning it off, or hashing names too

`record` has two settings of its own, separate from the report's `redact` and `redact-names`:

| Config key | Default | Flags | Does |
| ---------- | ------- | ----- | ---- |
| `ledger-redact` | `true` | `--ledger-redact`, `--no-ledger-redact` | Hash ids and worktree names in the lines `record` writes. |
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

With redaction on, the salt must live in the main checkout's `.tally/`. So
`record --ledger <path>` in a repository whose main checkout has no `.tally/` directory stops with
exit 1, and writes nothing:

```text
ledger redaction needs a .tally/ directory in the main checkout to hold its salt: create it, or pass --no-ledger-redact
```

Create `.tally/` in the main checkout, or pass `--no-ledger-redact` to write raw ids to that file.

### When the main checkout has not opted in

The salt is always looked for in the main checkout. If a git worktree's checkout has a `.tally/`
directory but the main checkout does not, `record` run in that worktree writes the worktree's
ledger **without** redaction, with raw ids and no `redaction` field, and says nothing about it.
Commit the opt-in, `.tally/salt` included, to the default branch, so that the main checkout has
`.tally/` too.

## Rewriting older lines

Lines written before redaction was on, or written with `--no-ledger-redact`, keep their raw ids.
`branch-tally ledger redact` rewrites them:

```sh
branch-tally ledger redact
```

```text
redacted 3 line(s), 3 already redacted, 1 unparseable left as-is
```

- It hashes every line that parses and has no `redaction` field, with the current salt, creating
  the salt if it is missing. A rewritten line then matches the line `record` writes for the same
  call, so the two collapse into one in a report.
- Lines that already carry a `redaction` field are left byte for byte as they are, whichever salt
  they were hashed with. Lines it can't parse are counted and left as they are, raw ids included.
  That includes a line whose `ts` is not strict RFC 3339 (see [Timestamps](cli.md#timestamps)).
- It runs even when `ledger-redact` is off in your config, because you asked for it. It uses
  `ledger-redact-names` from the config file, or `--ledger-redact-names` /
  `--no-ledger-redact-names`.
- It writes the new ledger to a temporary file next to it and replaces the ledger only when that
  succeeds. It does not commit anything.
- Without a `.tally/` directory in the main checkout, it changes nothing and prints
  `no .tally/ directory: nothing redacted (create it to opt in)`. With `--ledger <path>`, the same
  case is an error (exit 1), with the message shown in
  [An explicit `--ledger` needs `.tally/`](#an-explicit---ledger-needs-tally).

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
a call recorded once in each form appears under two different keys, so it is counted twice.
`report --from-ledger` and `project` say so on stderr:

```text
note: ledger holds 3 unredacted line(s) and 3 redacted line(s) under 1 salt(s); a call recorded in more than one form may be counted twice. Run `branch-tally ledger redact` to rewrite the unredacted lines
```

When the ledger has no unredacted lines, only lines under more than one salt, the note ends after
"counted twice": `ledger redact` can't fix that, because the raw ids behind a hash are gone.

### Combining the ledger with live transcripts

`report --from-ledger --with-live` hashes this machine's live calls with the local salt before
combining them with the ledger, so a call in both is counted once. When it can't do that, it says
why on stderr, and the total and the repository-wide counts may count a call twice:

```text
note: live calls could not be matched against the redacted ledger lines (<why>); the total and the repository-wide counts may count a call twice
```

| `<why>` | Meaning |
| ------- | ------- |
| `no salt in .tally/` | The ledger has redacted lines, but this checkout has no `.tally/salt`. Pull the commit that added it. |
| `the salt is unreadable` | `.tally/salt` exists but is not a valid salt. |
| `the local salt matches none of the redacted lines` | The redacted lines were hashed with a different salt than the one here. |
| `the local salt matches only some of the redacted lines` | The ledger holds lines under more than one salt, and only some are under the local one. |

## Report redaction is separate

Everything above is about the ledger file. What a report prints is set by `report`'s own
`--redact`, which is still **off** by default, including for `report --pr`. Without it, a report,
and the pull request body it writes, shows:

- the first eight characters of each session id (from a redacted ledger, of the hashed id);
- agent and skill names;
- worktree names and subagent labels;
- each session's working-time span, to the minute.

`--redact` replaces these with generic labels and dates. See [Redaction](cli.md#redaction).
