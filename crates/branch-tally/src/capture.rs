//! `capture`: the hook entry point. Reads one Claude Code hook payload from stdin, resolves the
//! branch (and, for a subagent stop, its worktree's branch) with git, and appends an id-only event
//! to the main checkout's `.tally/captures.jsonl`.
//!
//! Every failure is a fixed message: the payload and its cwd are never echoed (Governance III).

use std::io::Read;
use std::path::{Path, PathBuf};

use branch_tally_core::git::{Git, SystemGit, normalize_path};
use branch_tally_core::ledger::{Clock, SystemClock, append_captures};
use branch_tally_core::model::{CAPTURE_SCHEMA_VERSION, CaptureEvent, CaptureKind, Harness};
use branch_tally_core::sources::claude_code::hook::{
    HookEventKind, HookPayload, parse_hook_payload,
};

/// Hook payloads are small; refuse to buffer more than this.
const MAX_PAYLOAD_BYTES: u64 = 1024 * 1024;

pub(crate) fn run() -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_PAYLOAD_BYTES)
        .read_to_end(&mut input)
        .map_err(|_| "capture: could not read the hook payload from stdin".to_owned())?;
    let payload = parse_hook_payload(&input)
        .map_err(|_| "capture: the hook payload was not understood".to_owned())?;
    let dir = match &payload.cwd {
        Some(cwd) => PathBuf::from(cwd),
        None => std::env::current_dir()
            .map_err(|_| "capture: could not determine the working directory".to_owned())?,
    };
    let Some(event) = build_event(&payload, &dir, &SystemGit, &SystemClock)? else {
        return Ok(());
    };
    let root = SystemGit.main_checkout(&dir);
    append_captures(&root, &[event])
        .map_err(|_| "capture: could not write .tally/captures.jsonl".to_owned())?;
    Ok(())
}

/// The event to store for `payload`, or `None` for a hook moment the model has no kind for.
fn build_event(
    payload: &HookPayload,
    dir: &Path,
    git: &dyn Git,
    clock: &dyn Clock,
) -> Result<Option<CaptureEvent>, String> {
    let kind = match payload.event {
        HookEventKind::SessionStart => CaptureKind::SessionStart,
        HookEventKind::SubagentStop => CaptureKind::SubagentStop,
        HookEventKind::Stop => CaptureKind::Stop,
        HookEventKind::SessionEnd | HookEventKind::SubagentStart => return Ok(None),
    };
    let branch = git
        .current_branch(dir)
        .map_err(|_| "capture: not a git checkout, or git failed".to_owned())?;
    if branch.is_empty() || branch == "HEAD" {
        return Err("capture: HEAD is detached, so there is no branch to record".to_owned());
    }
    let worktree_branch = if kind == CaptureKind::SubagentStop {
        subagent_worktree_branch(payload, git, dir)?
    } else {
        None
    };
    Ok(Some(CaptureEvent {
        v: CAPTURE_SCHEMA_VERSION,
        harness: Harness::ClaudeCode,
        kind,
        session_id: payload.session_id.clone(),
        agent_id: payload.agent_id.clone(),
        branch,
        worktree_branch,
        ts: clock.now(),
    }))
}

/// The branch of the worktree this subagent ran in, never guessed: the worktree directory named
/// `agent-<id>`, else the payload cwd's own checkout when that is a linked worktree, else `None`.
fn subagent_worktree_branch(
    payload: &HookPayload,
    git: &dyn Git,
    dir: &Path,
) -> Result<Option<String>, String> {
    let main = git.main_checkout(dir);
    let worktrees = git
        .worktrees(&main)
        .map_err(|_| "capture: git worktree list failed".to_owned())?;
    if let Some(agent) = &payload.agent_id {
        let wanted = format!("agent-{}", agent.as_str()).to_lowercase();
        let found = worktrees.iter().find(|wt| {
            normalize_path(&wt.path)
                .file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase() == wanted)
        });
        if let Some(wt) = found {
            return Ok(Some(wt.branch.clone()));
        }
    }
    let top = git
        .toplevel(dir)
        .map_err(|_| "capture: not a git checkout, or git failed".to_owned())?;
    let key = |p: &Path| normalize_path(p).to_string_lossy().to_lowercase();
    if key(&top) == key(&main) {
        return Ok(None);
    }
    Ok(worktrees
        .iter()
        .find(|wt| key(&wt.path) == key(&top))
        .map(|wt| wt.branch.clone()))
}
