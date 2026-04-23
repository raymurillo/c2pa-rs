# /commit-changes

Create a git commit for general changes, fixes, or improvements that weren't part of a specific specification. This command handles the git workflow for committing ad-hoc changes with proper commit messages and verification.

## Variables

COMMIT_MESSAGE: $ARGUMENTS (optional - describe what was changed/fixed/added, auto-generated if not provided)
COMMIT_TYPE: auto-detected from message or can be specified (feat/fix/docs/style/refactor/test/chore)

## Execute

### Phase 1: Pre-commit Checks

1. Check if there are any uncommitted changes in the working directory
   - If no changes, exit with message "No changes to commit"
   - If changes exist, proceed to commit

2. Run enhanced pre-commit validation using justfile:
   - Check if justfile exists in project root
   - If justfile exists:
     - Execute `just fmt` to format code
     - Execute `just lint` to run linter
     - Execute `just test` to run unit tests
     - Execute `just check` to run type checking
   - If justfile doesn't exist, fall back to individual commands:
     - Execute `cargo fmt --check` to check code formatting
     - Execute `cargo clippy` to run linter
     - Execute `cargo check` to ensure code compiles
     - Execute `cargo test` to ensure tests pass
   - If any validation fails, exit with error and details

3. Optional CI skip flag:
   - Allow `--skip-ci` flag to bypass CI validation for emergency fixes
   - Log warning when CI is skipped

### Phase 2: Commit Preparation

1. Analyze changes to determine commit type and message:
   - If no commit message provided, analyze git diff to auto-generate message
   - Scan staged files and changes for keywords and patterns
   - `feat:` for new features, functionality, new files, new functions
   - `fix:` for bug fixes, error corrections, error handling improvements
   - `docs:` for documentation changes, README updates, comments
   - `style:` for formatting, missing semicolons, whitespace, code style
   - `refactor:` for code restructuring, function extraction, variable renaming
   - `test:` for adding or updating tests, test files, test coverage
   - `chore:` for maintenance tasks, dependencies, build files, config changes

2. Auto-generate commit message if not provided:
   - Analyze `git diff --cached` to understand what changed
   - Look for file patterns: `*.rs` (code), `*.md` (docs), `*test*` (tests), etc.
   - Identify key changes: new functions, error fixes, documentation updates
   - Generate descriptive message based on most significant changes
   - Example: "add error handling to camera initialization" or "update README with installation steps"

3. Format commit message:
   - If user provided type prefix (e.g., "fix: ..."), use as-is
   - If no type prefix, add appropriate type based on analysis
   - Format: `{type}: {COMMIT_MESSAGE}`
   - Ensure message is descriptive and follows conventional commits

3. Stage changes explicitly:
   - Execute `git diff --name-only` and `git ls-files --others --exclude-standard` to list modified and untracked files
   - Stage each relevant file by path: `git add <file1> <file2> ...`
   - Do NOT use `git add .` — parallel sessions share the repo root and `git add .` can capture another session's changes

4. Create the commit:
   - Execute `git commit -m "{formatted_commit_message}"`

### Phase 3: Post-commit Verification

1. Verify commit was created successfully:
   - Execute `git log --oneline -1` to show the latest commit
   - Display the commit hash and message

2. Show commit summary:
   - Execute `git show --stat` to show files changed
   - Display a summary of what was committed

3. Provide next steps:
   - Suggest running `git push` if ready to push to remote
   - Mention any follow-up tasks or related improvements

## Example Usage

```
/commit-changes "fix cursor jitter by improving smoothing algorithm"
/commit-changes "feat: add debug logging to eye detection"
/commit-changes "docs: update README with installation instructions"
/commit-changes "refactor: extract common camera utilities"
/commit-changes "test: add unit tests for pupil detection"
/commit-changes "chore: update OpenCV dependency to latest version"
/commit-changes --skip-ci "hotfix: emergency security patch"
/commit-changes
```

## Auto-detection Examples

- "fix cursor jitter" → `fix: cursor jitter by improving smoothing algorithm`
- "add debug logging" → `feat: add debug logging to eye detection`
- "update README" → `docs: update README with installation instructions`
- "extract utilities" → `refactor: extract common camera utilities`
- "add tests" → `test: add unit tests for pupil detection`
- "update dependency" → `chore: update OpenCV dependency to latest version`

## Auto-generation Examples (no message provided)

- New Rust files added → `feat: add new camera interface implementation`
- Error handling added → `fix: add error handling to camera initialization`
- README.md modified → `docs: update README with installation instructions`
- Test files added → `test: add unit tests for pupil detection`
- Code formatting changes → `style: format code with cargo fmt`
- Function extracted → `refactor: extract common camera utilities`
- Dependencies updated → `chore: update dependencies in Cargo.toml`

## Error Handling

- If no changes are staged: "No changes to commit"
- If no commit message provided: Auto-generate message based on git diff analysis
- If justfile commands fail: Enhanced error reporting with actionable suggestions
  - `just fmt` failure: "💡 Code formatting failed. Try running: just fmt"
  - `just lint` failure: "💡 Linting failed. Try: just lint to see specific issues"
  - `just test` failure: "💡 Unit tests failed. Try: just test to see which tests failed"
  - `just check` failure: "💡 Type checking failed. Try: just check to see compilation errors"
- If justfile not found: "⚠️ Justfile not found, using fallback Cargo commands"
- If build fails: "Build failed: {error details}"
- If tests fail: "Tests failed: {error details}"
- If git commit fails: "Commit failed: {error details}"

## Difference from /commit-spec

- `/commit-spec` is for changes that implement a specific specification file
- `/commit-changes` is for general improvements, fixes, or ad-hoc changes
- `/commit-spec` auto-generates messages from spec names
- `/commit-changes` requires a descriptive message and auto-detects commit type 