# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->


## Build & Test

_Add your build and test commands here_

```bash
# Example:
# npm install
# npm test
```

## Architecture Overview

_Add a brief overview of your project architecture_

## Conventions & Patterns

### 作業開始時にプロジェクトの記録を読み込む（必須）

このプロジェクトで作業を始めるときは、まず **Obsidian と Beads の記録を読み込んで**現状を把握すること。
コード・git 履歴だけでは分からない「なぜそうしたか」「どこで止まっているか」「過去の失敗・教訓」が
これらに記録されている。

- **Beads**: `bd ready` / `bd list` / `bd show <id>` / `bd stats` で課題状況を確認。
  `bd prime` で詳細コンテキスト、`bd memories <keyword>` で永続知識を参照。
- **Obsidian**（vault: `D:\Obsidianvault\ClaudeMemory\Projects\Script2Voice-Rust版\`）:
  - 進捗ノート `進捗.md` — マイルストーン・完了内容・設計メモ・ハマりどころ
  - 教訓ノート `失敗・教訓ログ.md` — 過去の失敗・エラー・「やれば良かったこと」
  - Obsidian 起動中は `obsidian` CLI（`obsidian read path="..."` 等）で読む。

### 失敗・教訓の記録（必須）

プロジェクトの進行にあたって遭遇した**失敗・バグ・エラー・「やれば良かったこと」**は、
発生のたびに Obsidian の専用ノートに記録すること。進捗そのものは進捗ノートに、
うまくいかなかったことと次に活かす学びはこの教訓ノートに分けて蓄積する。

- **記録先（Obsidian vault）**: `D:\Obsidianvault\ClaudeMemory\Projects\Script2Voice-Rust版\失敗・教訓ログ.md`
  （新しい知見はノート先頭付近に追記）
- **進捗ノート**: `D:\Obsidianvault\ClaudeMemory\Projects\Script2Voice-Rust版\進捗.md`
- **1件あたりの記録項目**: ①何が起きたか（症状）②根本原因 ③やれば良かったこと/教訓（次回への一般化）
  ④関連 Beads / コミット
- Obsidian 起動中は `obsidian` CLI（append 等）が使える。未起動時は上記パスに直接書き込む。
