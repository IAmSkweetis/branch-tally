# Reference pages

Each page records the decisions and gotchas for one area, the things **not** recoverable by reading
the code. None of them are loaded into context automatically.

Read a page when the task at hand is actually about its area, before making changes there. Don't
read one proactively or "just in case": that is what keeps them cheap to have.

When you change what a page describes, update the page in the same commit. A page that disagrees
with the code is worse than no page, because people trust it.

## The pages

| Page | Read it when |
| ---- | ------------ |
| [project_layout.md](project_layout.md) | Adding a file and deciding where it goes: which crate, which module, where its tests and fixtures live. |
| [dev_workflow.md](dev_workflow.md) | Naming a branch, writing a commit message, running or adding a `just` recipe, opening a PR, or asking what has to be green before something lands. |
| [testing.md](testing.md) | Writing or running a test, adding a fixture, touching a snapshot, or checking parity against the Python reference. |
| [agent_workflow.md](agent_workflow.md) | Dispatching a subagent, running work in parallel worktrees, reading an agent report, or routing an escalation. |
| [agents.md](agents.md) | Deciding which agent owns a file, or what a given agent may write. |
| [release.md](release.md) | Cutting a release, how versions are chosen, or how artifacts are built and published. |
| [transcript_format.md](transcript_format.md) | Parsing Claude Code transcripts, or attributing records to branches, worktrees or subagents. |

## Adding a page

Write it here, add a row above, and add a row to the table in `AGENTS.md` so an agent knows it
exists. Nobody reads a page that nothing points to.

## Where the rest lives

Not everything is a reference page:

- **`AGENTS.md`**: the standing rules an agent should follow without being asked. That covers
  governance, the tech stack, the test command and the workflow in brief. Claude Code reads it
  natively.
- **`docs/decisions/`**: architecture decision records, one per significant choice, numbered and
  never rewritten. A later decision supersedes an earlier one rather than editing it. The first is
  the language decision (#1).
- **GitHub issues**: the roadmap (#16 and its sub-issues). Planned work lives there, not in a
  `ROADMAP.md`.
- **`.claude/skills/`**: procedures rather than decisions.
  - `feature` orchestrates a piece of work from interview to merge.
  - `worktree-bootstrap` prepares a worktree.
  - `rust-tests` covers running and writing tests.
  - `create-gh-pr` opens the PR.
  - `worktree-cleanup` closes the work out once it has landed.
- **`.claude/agents/`**: the agent definitions themselves, each carrying its own binding rules.
- **`.claude/agent-report-contract.md`**: the shape every agent's report must follow.
