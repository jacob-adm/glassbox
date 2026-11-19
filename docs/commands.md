# Commands

## `glassbox reach`

```
glassbox reach [--json] [--strict] [--home DIR]
```

Reports what a process started like this one could read. Run it from the terminal, editor or
service you start agents from: an agent started there inherits the same user, permissions and, on
macOS, the privacy grants of the responsible app.

| Option | Effect |
|---|---|
| `--json` | Print JSON instead of the text report |
| `--strict` | Exit 1 if any sensitive location is `readable` |
| `--home DIR` | Resolve locations under `DIR` instead of `$HOME` |

### What it does to each location

| Kind | Probe | Never |
|---|---|---|
| Directory | `opendir` | lists entries |
| File | `open` read-only | reads bytes |
| Anything | – | writes, creates or deletes |

A symlink is followed, as an agent's open would follow it, so the result is for its target.

### Results

| Result | errno | Meaning |
|---|---|---|
| `readable` | – | The open succeeded |
| `denied-privacy` | EPERM | A privacy control or sandbox refused it (macOS TCC, Seatbelt, SIP) |
| `denied-permissions` | EACCES | File permissions or ACLs refused it |
| `absent` | ENOENT | Nothing at that path |
| `error-N` | N | Any other error |

### Text output

```
Process chain (nearest first):
    40732  /bin/zsh
    92812  /Applications/iTerm.app/Contents/MacOS/iTerm2
        1  /sbin/launchd
Privacy grants on macOS follow the responsible app: iTerm.app

! readable             credentials       ~/.ssh
! readable             private data      ~/Library/Mail
  readable             protected folder  ~/Documents

2 sensitive location(s) readable (marked !); 11 not present.
```

Absent locations are counted but not listed.

### JSON output

```json
{"chain":[{"pid":40732,"command":"/bin/zsh"}],
 "responsible_app":"iTerm.app",
 "findings":[{"category":"credentials","sensitive":true,"path":"~/.ssh","reach":"readable"}]}
```

`responsible_app` is `null` when no `.app` bundle is in the chain (always on Linux).

### Exit status

| Status | When |
|---|---|
| 0 | Report printed. With `--strict`, also: no sensitive location readable |
| 1 | `--strict` and at least one sensitive location readable |
| 2 | Usage error, or `HOME` unset without `--home` |

### Locations

Paths are relative to the home directory. Sensitive locations count towards `--strict`.

| Category | Sensitive | macOS | Linux |
|---|---|---|---|
| credentials | yes | `.ssh`, `.aws/credentials`, `.config/gcloud`, `.azure`, `.kube/config`, `.docker/config.json`, `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`, `.config/gh/hosts.yml`, `.gnupg`, `.password-store`, `Library/Keychains` | the same, with `.local/share/keyrings` instead of `Library/Keychains` |
| history | yes | `.zsh_history`, `.bash_history`, `.python_history` | the same |
| private data | yes | `Library/Mail`, `Library/Messages`, `Library/Safari`, `Library/Application Support/AddressBook`, `Library/Calendars`, `Pictures/Photos Library.photoslibrary` | `.thunderbird`, `.local/share/evolution` |
| browser profile | yes | Chrome, Firefox and Brave under `Library/Application Support` | `.mozilla/firefox`, `.config/google-chrome`, `.config/chromium`, `.config/BraveSoftware` |
| protected folder | no | `Desktop`, `Documents`, `Downloads`, `Library/Mobile Documents` | – |

### Limits

- Read access only. Write access is not probed, because a non-destructive write test is not
  possible for every location.
- A sandbox applied *to the agent* (for example sandbox-runtime or nono) is not seen unless glassbox
  runs inside the same sandbox. Run it there to check the policy.
- On Linux, confinement by AppArmor, SELinux or Landlock shows up only as a refused open; glassbox
  does not read the policy.
