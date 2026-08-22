# Security Policy

## Reporting a vulnerability

Please report security issues through GitHub's private vulnerability reporting:
open the **Security** tab on this repository and choose **Report a vulnerability**.

That keeps the report private until a fix is available. Please do not open a public
issue for a security problem.

This is a hobby project maintained by one person. Reports will be acknowledged as
soon as reasonably possible, but there is no guaranteed response time and no bounty.

## Supported versions

Only the most recent release receives fixes.

## Scope

The areas most worth looking at:

- **The elevated autostart path** (`src/autostart.rs`). Registering the Task
  Scheduler entry runs with administrator rights, and the task definition is written
  to a temporary file that `schtasks.exe` then reads.
- **Parsing of content that may come from a third party**: `config.ini`,
  `lang\*.ini`, and theme folders under `theme\`.
- **The log** (`log.txt`), which records window titles produced by other processes.

## Known limitations, by design

The following are documented in the README and are not treated as vulnerabilities.

- **The executable's directory is the trust boundary.** Anyone who can write to that
  directory can change how the program behaves — and if elevated autostart has been
  registered, that behaviour runs as administrator. Install it somewhere only
  administrators can write to, or keep it under your own profile and treat the
  directory as you would any other executable location.
- **Themes and language files are third-party content.** Their names and paths are
  validated and their size is capped, but the contents of `.ico`, `.ani` and `.wav`
  files are parsed by Windows, not by this program.
- **UAC consent prompts are unreachable, deliberately.** They are drawn on the secure
  desktop, and the pause they force is a security feature.

For reference, the program never clicks, never sends input to other windows, makes no
network connections, and collects no telemetry. It writes only `config.ini` and
`log.txt`, and launches only `schtasks.exe`, `notepad.exe`, and itself — each by an
absolute path from the system directory.
