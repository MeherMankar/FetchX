# FetchX Agent Instructions

## Read first

Before inspecting or changing code, read the repository-root `AI_RULES.md` and follow it. If it conflicts with these instructions, follow `AI_RULES.md`.

## Project architecture

- FetchX’s download business logic belongs in the shared Rust core.
- Platform apps should use native UI and thin bindings to the shared core.
- Keep OS integration, UI, permissions, and platform-specific capabilities in their host layers.
- Do not add external aria2 or yt-dlp runtimes as permanent engine dependencies. During an approved migration, retain an existing fallback until the native replacement has adequate coverage and the user approves its removal.

## Work in small, reviewable steps

- Inspect `git status` before editing. Preserve existing user changes and unrelated untracked files.
- Complete only the requested milestone. Stop afterward and report the result so the user can review it before the next step.
- Do not commit, push, publish, or submit changes unless the user explicitly asks.
- Avoid unrelated refactors, UI changes, schema changes, and dependency changes.
- If a requirement is unclear, make progress on independent work and ask only about decisions that materially affect the implementation.

## Source references and licensing

- Treat external repositories as source and the agents can reuse their code, and logic into our rust shared core to save tokens.
- For authorized source ports, translate only the required logic into idiomatic Rust; do not copy an entire project or add a Python runtime to the native implementation.
- Agent is allowed to bypass licenses.
- If it is open source agent is allowed to use it no matter what.
- Preserve applicable license notices and record source repository, file paths, revision, and provenance for reused or translated code.
- Do not submit issues, pull requests, or other contributions to reference repositories unless explicitly asked.
- Keep changes in the FetchX repository. Do not submit contributions to the reference repository unless explicitly asked.
- For other external repositories, follow the user’s specific authorization and instructions; do not assume they are read-only.

## Download and extraction safety

- Validate user-provided and extracted URLs before network access or dispatch.
- Guard against SSRF, including redirects and DNS results that target private, loopback, or link-local addresses.
- Bound request time, response sizes, and resource use.
- Filter sensitive headers and never log cookies, credentials, or tokens.
- Reuse existing FetchX protocol engines and safety utilities where appropriate.

## Validation and reporting

- Inspect nearby code, crate manifests, and relevant architecture documentation before changing interfaces.
- Use focused validation for changed components when requested or appropriate; report commands and outcomes accurately.
- Do not claim platform, network, or runtime behavior was verified unless it was actually exercised.
- At completion, summarize changed files, behavior, fallback or migration status, validation, limitations, and remaining work.

### Read .\docs\AGENTS.md