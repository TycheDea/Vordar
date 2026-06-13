# Integration Toolkit

This folder contains executable assets to enforce the workflow defined in:
- `INTEGRATION-PLAN.md`
- `EXECUTION-PLAN.md`
- `CURSOR-IMPROVEMENTS.md`

It is generic and reusable across repositories.

## Folder Structure
- `templates/` prompt and packet templates
- `trackers/` working state and decision logs
- `scripts/` PowerShell automation for loop enforcement

## Quick Start
1. Initialize baseline files:
   - `powershell -NoProfile -ExecutionPolicy Bypass -File .\integration\scripts\init-integration.ps1`
2. Start a task loop:
   - `powershell -NoProfile -ExecutionPolicy Bypass -File .\integration\scripts\start-task.ps1 -Task "my_task"`
3. Build a debugger prompt payload:
   - `powershell -NoProfile -ExecutionPolicy Bypass -File .\integration\scripts\build-debug-prompt.ps1`
4. After applying a fix, update loop state:
   - `powershell -NoProfile -ExecutionPolicy Bypass -File .\integration\scripts\update-iteration.ps1 -ErrorCode E0308`

## Enforcement Rules Implemented by Scripts
- Requires structured packets (`state`, `decisions`, `error`, `cursor_diff`, `mcp_check`)
- Enforces escalation after 2 repeated errors
- Enforces debugger output shape (`CAUSE:` and `FIX:` required)

## Notes
- Scripts are non-destructive and create missing files when needed.
- All paths are repo-relative from where the script is executed.
