# Containing effects, not tools

### A position on security for `jinn`, and the place of Microsoft Execution Containers in it

> **Abstract.** `jinn` is a terminal agent harness: it turns model output into
> shell commands, file edits, and network calls that run on the user's machine
> with the user's full authority, and it has no approval gate. This paper takes
> a position on what security means for such a program. The position has four
> claims. First, the unit of security policy is the *effect* the model causes (a
> process, a file write, a change to the harness's own state), not the tool that
> produced it. Second, a census of `jinn` at a pinned commit shows its most
> damaging effects are in-process: file tools resolve paths lexically with no
> confinement, and every spawned process inherits the parent's credentials,
> so no process sandbox, MXC included, closes the worst two kill chains on its
> own. Third, Microsoft Execution Containers (MXC) are the strongest
> available enforcement for process effects on Windows, the one platform with
> no native alternative, and their learning mode is the only deny-report
> authoring loop in the field; measured on Windows 11 26H2 with `mxc-sdk`
> 1.0.1, the backend enforces filesystem and network policy natively with no
> host mutation. Fourth, therefore, MXC should be adopted as *one backend* of a
> harness-owned policy primitive, beside an in-process file-tool guard and
> harness-authority rules, and not as the security architecture itself. The
> paper includes the census, the threat model, measured backend behavior, and
> the objections to its own position.

---

## 1. Position

**C1 — The effect is the unit of security.**
A model does not "run a tool"; it causes something to happen. Every such thing
is an effect, and effects come in three classes: **process** (a spawn in the
harness), **filesystem** (an in-process I/O call), and **harness** (a change to
the harness's own state, such as enabling an MCP server). A policy enforced in
one class leaves the other two open. This is the structural claim of the paper,
and §3 supplies the evidence.

**C2 — `jinn`'s dominant risks are in-process.**
The file tools resolve paths lexically with no canonicalization and no
confinement; the environment is inherited by every child unfiltered; there is
no approval gate. Two of the three kill chains in §4 are therefore invisible to
any process sandbox. A harness that adopts only a sandbox has bought the
cheapest third of its problem.

**C3 — MXC is the right Windows process backend, adopted as a backend only.**
Measured on Windows 11 26H2 (build 26300.9711), the `processcontainer` backend
selects the strongest enforcement tier, enforces filesystem deny and grant
natively, denies network egress by default, and needs no host mutation or
elevation. Its gaps (no interactive terminal, no Git Bash, an environment
allowlist floor) are real but survivable, because the architecture in §5 does
not ask MXC to be everything.

**C4 — Honesty is a design constraint, not a preference.**
"Enforce" must mean enforced. A surface the policy does not cover is either
withheld or documented as uncovered; a denial surfaces as a tool error the model
can read; the whole boundary is described as blast-radius reduction, not as
defense against a capable adversary.

---

## 2. Background and scope

### 2.1 The problem, stated plainly

An agent harness runs shell commands on the user's behalf. Those commands are
written by a language model, which makes them **semi-trusted by construction**:
usually correct, occasionally broken, and in the worst case steered by hostile
text that leaked into the context that produced them. Uncontained, every one of
those commands runs as the user. It can read the user's credentials, phone
home, delete things, and exfiltrate keys.

The traditional defenses do not apply. An antivirus signature database has no
opinion on `rm -rf`. A permission dialog per command trains the user to click
"Yes" within a week, and prompt injection exists precisely to exploit that
reflex. Containment inverts the default: the workload gets nothing unless the
policy grants it, and the operating system does the refusing.

### 2.2 `jinn`'s starting point

`jinn` ships with none of this. Tool execution is ungated: the only
confirmations in the tree are UI interactions. The file tools (`read`, `write`,
`edit`, `save_plan`) do their I/O inside the harness process with lexical path
resolution and no confinement. The `bash`, `powershell`, and
`interactive_term` tools spawn real processes. The environment, including
provider API keys resolved at startup, is inherited by every child unfiltered.

The field notes behind this paper: ["Enter MXC"](https://jeff-mitchell.dev/blog/enter-mxc-what-process-containers-even-are/)
and ["Enter MXC, Part 2: The Fence I Didn't Build"](https://jeff-mitchell.dev/blog/enter-mxc-part-2-the-fence-i-didnt-build/)
on jeff-mitchell.dev. The second records the finding that reshaped this
position: a weekend prototype wrapped the `bash` tool in an MXC container, and
it worked, but the audit that planned it searched for spawn sites, so it found
spawn sites, and missed that the file tools and the conversation channel are
the whole game. The prototype was abandoned rather than ship a fence nobody
could state the boundaries of. This paper is the boundary statement that
prototype lacked.

### 2.3 Windows and MXC

On Linux, the containment problem has a deep bench: namespaces, seccomp,
Landlock, Bubblewrap, decades of overflow. Windows has AppContainer, a mature
mechanism that was never exposed as a tool for this workload class: a process
is launched under a derived sandbox identity so the kernel's ordinary access
checks stop recognizing it. **Microsoft
Execution Containers (MXC)** is Microsoft's open-source, policy-driven sandbox
SDK for running untrusted code (model output, plugins, and tools) on Windows,
Linux, and macOS, and it is the first way to reach AppContainer. It enforces a declared policy
at the OS boundary: the workload is launched into a restricted identity and
cannot exceed what the policy grants. Version 1.0 shipped on 2026-10-07; this
paper measures 1.0.1. Upstream source and documentation:
`github.com/microsoft/mxc`.

The sandbox evaluated here is MXC; the position is not "MXC". On Linux the same
policy maps to Bubblewrap, which MXC also wraps but which a harness can call
directly.

### 2.4 What this position does not claim

- **Not defense against a capable adversary.** Microsoft states that MXC
  profiles are not security boundaries, and the 1.0 release removed that
  disclaimer without publishing an audit. Backend-specific limits remain
  documented (§6.2). One sentence to carry into every discussion: **this is
  containment of convenience and blast-radius reduction.**
- **Not a claim that containment covers the harness.** A malicious MCP server,
  a malicious user, a compromised harness binary, and a compromised model
  provider are out of scope; the last of these is definitionally unreachable,
  since everything sent to a provider is disclosed to it.
- **Not a rollout.** §5.3 orders the work, but nothing in this paper is
  implemented. The census describes `jinn` at `main` (`687cfb17`).

---

## 3. Census: what the harness can cause (evidence A)

The model's authority is the set of effects the harness can produce. This
section enumerates them from the tree at `main` (`687cfb17`), production code
only.

### 3.1 Process spawns

The "author" tier is who wrote the command: **tier 1** means the model (through
a tool), **tier 2** means the user (through config or workflow), **tier 3**
means nobody: infrastructure choosing its own helper binaries. Tier 1 is what
containment must watch. (These tiers are unrelated to MXC's containment tiers
in §6.)

| Site | Author | Tier |
| --- | --- | --- |
| `crates/slices/jinn-tools/src/bash.rs:434` — `spawn_shell_command` | model | 1 |
| `crates/slices/jinn-tools/src/powershell/execute.rs:83` | model (Windows) | 1 |
| `crates/slices/jinn-term/src/pty_session.rs` — the terminal device behind `interactive_term` | model | 1 |
| `crates/slices/jinn-tools/src/grep.rs:167` — spawns `rg` | model controls pattern and path args | 1 |
| `crates/jinn-llm-support/src/image_convert/converter.rs:116` | model-influenced input | 1 |
| `crates/slices/jinn-session-lifecycle/src/command_runner.rs:62,221,373` | user (config / workflow TOML) | 2 |
| `crates/jinn-mcp/src/client.rs:187,291` — stdio and HTTP MCP servers | config; third-party binary | 2/3 |
| `crates/jinn-tui/src/run.rs:247` — `sh` for `$EDITOR` | user | 2 |

Tier 1 is five sites. The commands a model authors reach the OS through all
five, not only through the `bash` tool.

`grep` is worth calling out: it spawns `rg` to walk paths the model chose, so a
policy that grants the repository read-only still permits traversal of anything
inside that grant. The unit of control is the *path grant*, not the tool.

### 3.2 Filesystem effects

`read`, `write`, `edit`, `save_plan`, and `read_image` all resolve their path
through one function, `crates/slices/jinn-tools/src/tool_paths.rs`:

```rust
/// Resolution is purely lexical. It does not canonicalize, expand `~`, or touch
/// the filesystem, so it cannot fail and cannot be surprised by a symlink.
pub(crate) fn resolve_path(path: &str, cwd: &MarkedPathBuf<Absolute>) -> MarkedPathBuf<Absolute> {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        MarkedPathBuf::<Absolute>::new(p).expect("checked absolute above")
    } else {
        cwd.join(p).expect("absolute parent joins a bare candidate")
    }
}
```

Consequences, all of them intended by that comment and all of them load-bearing
for this paper:

- **Absolute paths pass through unchanged.** Any file the user can read or write
  is reachable: `~/.ssh/id_ed25519`, `~/.aws/credentials`, the browser profile.
- **`~` is not expanded.** The model writes the literal path, which is less
  surprising but no safer.
- **No canonicalization.** `..` and symlinks/junctions are not resolved, so a
  lexical check would be trivial to defeat even if one existed.
- **No confinement to the session directory.** There is no sandbox root concept
  in the harness.

`write` then creates missing parents before writing:

```rust
if let Some(parent) = resolved.parent()
    && !parent.as_os_str().is_empty()
    && let Err(e) = tokio::fs::create_dir_all(parent).await
```

So the write tool will create a directory tree anywhere the user can write, then
write into it.

### 3.3 Network egress

| Client | Location | Reaches |
| --- | --- | --- |
| provider HTTP (`reqwest`) | `crates/jinn-provider` — `anthropic/`, `openai_compat/`, `google/` | provider endpoints and any configured `base_url` |
| model catalog | `crates/slices/jinn-model-market` | OpenRouter |
| MCP transports | `crates/jinn-mcp` | arbitrary URL; also binds a local port |
| Discord | `crates/slices/jinn-discord` | Discord gateway |
| `openrouter:web_search` | model-callable tool | the open web |

`crates/slices/jinn-citations` performs no I/O: it detects citable URLs by
shape. It is not an egress surface.

The provider client is the surface that matters most, and it is the one no
process sandbox can reach.

### 3.4 Credentials

`crates/slices/jinn-boot/src/env_init_actor.rs` resolves keys from the process
environment at startup: every provider's configured `api_key_env`, every `${VAR}`
referenced by an MCP server's headers, and `DISCORD_BOT_TOKEN` in
`crates/slices/jinn-discord/src/backend/spawn.rs:45`.

**No spawn site filters the environment.** There is no `env_clear`, no `.env(…)`,
and no `envs(…)` call in `bash.rs`, `powershell/execute.rs`, `command_runner.rs`,
or `jinn-mcp/src/client.rs`. Every model-authored command, every lifecycle
script, and every MCP server inherits the full parent environment, including
the keys the env-init actor just resolved.

### 3.5 Harness effects (no process, no file)

- `capabilities_apply`: model-callable, and it enables tools, skills, and MCP
  servers, which is what causes the harness to open connections and spawn
  servers.
- `spawn_subagent`, `tell_session`, `notify_parent`, `load_attendant`: spawn,
  drive, and steer other sessions.
- `run_lifecycle`: runs a shell command from a lifecycle script.
- `skill`: loads markdown into the model's context.

### 3.6 Persistence: delayed execution

Combined with §3.2, the write tool can place payloads that execute later, with
the user's authority and outside any fence:

| Target | Executes when |
| --- | --- |
| `.git/hooks/*` | the user commits |
| `~/.gitconfig` — `core.hooksPath`, `core.pager`, `alias.*` | the user runs git |
| `.envrc` | the user allows the directory in `direnv` |
| `jinn.toml` / `providers.toml` — an added MCP server | jinn next launches |
| `~/.agents/skills/*` | the next session builds its system prompt |
| attendant TOMLs | the attendant is next loaded |

The config rows are qualitatively different from the git rows: they extend the
model's own authority across a restart.

### 3.7 What the census establishes

1. **There is no approval gate.** Tool execution is ungated; the only
   confirmations in the tree are UI interactions (chat input popup, cwd entry).
   Containment is therefore the only control available, which raises the cost of
   getting it wrong.
2. **The most damaging surfaces are in-process.** Of the effect surfaces above,
   only §3.1 and §3.3's harness-side sockets are things a process sandbox can
   see.
3. **The harness is an effect surface in its own right.** `capabilities_apply`
   and a write to `jinn.toml` both let the model widen its own fence.

---

## 4. Threat model (evidence B)

### 4.1 Assets

| # | Asset | Reached by |
| --- | --- | --- |
| A1 | Home directory — SSH keys, `.aws`, `.config`, browser profiles | `read`, `write`, `bash`, `rg` |
| A2 | Provider API keys | the environment of every child process |
| A3 | Other credentials — Discord token, MCP header tokens | same |
| A4 | Network reach, both directions | provider HTTP, `bash`, MCP, web search |
| A5 | Harness integrity — config, skills, prompts, session DB | `write` plus a restart |
| A6 | Sibling sessions and the parent agent | `tell_session`, `spawn_subagent` |
| A7 | The user's belief that tool output is trustworthy | the conversation channel |

### 4.2 Adversaries

| # | Adversary | In scope |
| --- | --- | --- |
| T1 | Model error | yes — the main case |
| T2 | Opportunistic prompt injection | yes |
| T3 | Malicious MCP server / skill / plugin | partially — blast radius only |
| T4 | Malicious dependency in a repository | yes, via (1) and (2) |
| T5 | Attacker targeting the container mechanism | **no** |

(The `T1`…`T5` labels name adversaries in this section only. MXC's containment
tiers in §6 are unrelated and also use T-numbers.)

### 4.3 Injection entry points

Web search results, fetched URLs, MCP tool results, every file `read` returns,
session history from sibling sessions, skill and persona markdown, workflow
TOMLs, MCP tool *descriptions*, and citation payloads.

The property that matters: **the model reads all of these at full authority**,
because reading is how it works. Containment must assume the model is already
cooperating with the attacker.

### 4.4 Kill chains

**K1 — exfiltration through the conversation.**
Injection instructs the model to read a credential file and include it in a
summary. `read` returns it in-process; the provider client ships it out. A
network-deny inside a process fence never observes this, because the process
that reads the key is jinn itself. **Owner: the `read` path policy.**

**K2 — delayed execution through a file.**
Injection gets `write` to drop a git hook, or to add `core.hooksPath` to
`~/.gitconfig`. The fence never sees it; the user's next commit executes it.
The damage is delayed and arrives through a legitimate-looking action: policy
enforcement blocks `rm -rf ~`, the payload rides in as a file instead, and the
user's own next commit detonates it. **Owner: the `write` / `edit` path
policy.**

**K3 — authority expansion through the harness.**
Injection calls `capabilities_apply` to enable a network MCP server, or writes
an MCP entry into `jinn.toml`. The harness then makes the connection on the
model's behalf on a later turn. A process container cannot stop a socket jinn
opens itself. **Owner: the harness's authority model.**

Each chain has a different owner. **None of the three is a process sandbox.**
This is the load-bearing sentence of the paper: it is why C2 holds, and it is
what distinguishes "adopt a sandbox" from "adopt a policy".

---

## 5. The answer: one policy, three enforcers

### 5.1 What a process sandbox covers

| Surface | Covered by a process sandbox | Note |
| --- | --- | --- |
| `bash` tool | yes | the primary case |
| `powershell` tool | yes | |
| `interactive_term` PTY | **no** | `processcontainer` rejects PTY mode outright — verified, see §6.1. |
| `grep` → `rg` | yes, when spawned in-fence | path grants still decide reach |
| image converter | yes | model-influenced input, long exploit history |
| lifecycle commands | yes | Tier 2 author; still an effect |
| MCP server *process* | yes | not its traffic, and not its results |
| `read` / `write` / `edit` | **no** | in-process |
| provider HTTP | **no** | |
| session store | **no** | |
| `capabilities_apply` | **no** | |
| `tell_session` / `spawn_subagent` | **no** | |
| config writes | **no**, unless the write tool is itself fenced | |
| environment inheritance | **only if the allowlist is built** | a fence is only as good as what is passed into it |

A process sandbox closes the process column. On its own it does not close K1,
K2, or K3.

### 5.2 Target architecture

The policy is the primitive; a sandbox backend is one of its implementations.
The names below are proposed components, not existing ones:

```text
EffectPolicy            a data type describing effects: platform-neutral,
                        defined in a -msg crate (§2 of the style guide)
  filesystem:  read / write / deny paths
  network:     egress / ingress, deny by default
  limits:      timeout, memory, process count
  env:         allowlist, replacing inheritance
        |
        +-- MxcRunner     #[cfg(windows)]  process effects: bash, powershell,
        |                                   PTY, rg, converter, lifecycle, MCP
        +-- BwrapRunner   #[cfg(unix)]     process effects: the same set
        +-- ToolGuard     all platforms    filesystem effects: read, write,
                                          edit, save_plan, read_image
```

Three properties follow, and they are the point of the shape:

1. **K1 and K2 are closed by `ToolGuard`**, which has nothing to do with any
   sandbox and should ship first: canonicalizing, symlink- and junction-aware
   resolution confined to the policy's grants, consulted by every filesystem
   tool, with denial surfaced as a visible tool error rather than a silent
   retry.
2. **K3 is a harness-authority decision.** While a policy is in force,
   `capabilities_apply` and writes to `jinn.toml`, `providers.toml`, skills, and
   attendant TOMLs must be refused or require an out-of-band approval. Otherwise
   the model widens its own fence across a restart.
3. **The environment allowlist is built by the harness**, at the spawn boundary:
   `spawn_shell_command`, `spawn_powershell_command`, `run_command`, and
   `McpClient::connect`. MXC replaces the default environment when a request
   supplies one, but containment must not depend on a backend's default for a
   property this important.

The model's contract does not change. It continues to speak bash, and the tool
schema and output shape are untouched. Which shell runs *inside* the fence is an
implementation detail of the runner that nothing outside the runner knows about.

That seam matters because of history: an earlier prototype wrapped the `bash`
tool in an MXC container and worked, but MXC could not host Git Bash (§6.2), so
every model command had to be re-spelled through `cmd` to get one, and the
container's calling convention had leaked into the model's contract. The lesson
is recorded here so the runner boundary prevents the leak structurally rather
than by discipline.

### 5.3 Rollout order

1. **`ToolGuard` and the environment allowlist.** Platform-neutral, closes K1 and
   K2 today, depends on no sandbox. Ship first.
2. **Harness-authority rules for K3.** Config and capability writes refused
   under policy.
3. **`BwrapRunner` on Linux.** One primitive, no tiers, no host mutation, no
   elevation.
4. **`MxcRunner` on Windows.** Filesystem and network policy, enforcement first,
   learning mode as the authoring loop where the host supports it.
5. **Named-host egress.** Requires a proxy; CIDR allowlists are the wrong tool
   for named hosts, because a shared CDN's ranges cover far more than one host.

### 5.4 What the `interactive_term` gap means

The PTY cannot enter a Windows process container (§6.1), and MXC's engine-level
message says it does not yet support PTY spawning for that backend. So on
Windows the interactive terminal is **outside the fence** in v1, and the two
honest options are:

1. **Withhold `interactive_term` while an enforcing policy is active.** The tool
   disappears from the registry; the model reads an error saying the terminal is
   unavailable under containment. This is the only option that keeps "enforce"
   honest.
2. **Keep the terminal and say so.** It becomes a documented hole, listed in the
   coverage table, and the policy is labelled as process-level for batch tools
   only.

Do not ship option 2 while calling the mode "enforce". A hole nobody can see in
the docs is the failure this paper exists to prevent.

A third path is worth prototyping before committing: MXC supports PTY on
Bubblewrap, LXC, and Seatbelt direct execution, so the Linux runner keeps
`interactive_term` inside the fence on day one. Windows loses the terminal only
if `processcontainer` remains the backend.

---

## 6. Measured MXC behavior (evidence C)

MXC's Windows `processcontainer` backend resolves its enforcement mechanism at
runtime, choosing the strongest the host offers:

- **tier 1 "base-container"**: enforced natively by the OS process-security API
  (PSEC, `processmodel.dll`; Windows 11 25H2+ with the Process Isolation
  feature enabled). No host filesystem changes; native learning-mode capture.
- **tier 2**: AppContainer plus a BFS helper. Not compiled into shipping MXC
  builds; treat as unavailable.
- **tier 3**: AppContainer plus host-path DACL augmentation. The universal
  fallback: enforces filesystem policy by applying deny ACEs to real host paths
  for the duration of a run and removing them on exit.

The probe function (`mxc_sdk::v1::probe`) reports which tier a host selected and
`needs_dacl_augmentation`: `false` means tier 1, `true` means the DACL path.

### 6.1 What was verified on this machine

Measured with `mxc-sdk` 1.0.1 on Windows 11 **26H2, build 26300.9711**, where
`processmodel.dll` is present and the probe reports:

```text
tier                            base-container        (tier 1)
needs_dacl_augmentation         false
capabilities                    CaptureDenials, FilesystemDeniedPaths,
                                FilesystemEnumeratePaths,
                                IngressHostLoopbackAllow,
                                IdentitylessLoopbackProxy
native_capture_available        true
guarded_capture_available       false
```

Tier 1 is the good tier: filesystem policy is enforced natively by the OS's
process-security environment (PSEC), no host ACLs are touched, host preparation
and elevation are not required, and learning mode capture is native rather than
an elevated trace-helper fallback ("guarded WPR"). The measurements below were
taken there.

| Question | Result |
| --- | --- |
| PTY on `processcontainer` | **Rejected.** `UnsupportedContainment`: "the mxc engine does not yet support PTY spawning for the 'processcontainer' backend". The Rust README says create-and-run PTY is available for IsolationSession, Bubblewrap, LXC, and Seatbelt only; the Node README's "all backends except WSLc" is wrong. |
| Git Bash in-fence | **Dies.** `Exited(-1073741502)` = `0xC0000142`, `STATUS_DLL_INIT_FAILED`, with no output. `cmd.exe` in the same fence exits 0. Confirms the MSYS2/Cygwin namespace failure is not tier-specific. |
| Environment, omitted | **Not inherited.** A variable exported in the parent (`JINN_PROBE_SECRET`) read back as empty inside the container. Omission means the backend default, which is not the parent environment. |
| Environment, supplied | **Rejected without `SYSTEMROOT` and `LOCALAPPDATA`.** `PolicyValidation`: the supplied environment is used verbatim when `inherit_default_environment` is false, and those two names are presence-only requirements on Windows. An empty allowlist cannot launch. |
| Filesystem, no policy | **Deny-all, including the working directory.** Reading a sibling file, the project's own file, and `~/.gitconfig` all returned `Access is denied`; writing into the working directory also failed and produced no file. |
| Filesystem, `readwrite_paths` | **Works, and is confined.** Reading and writing inside the grant succeeded; reading a sibling directory outside it was denied. |
| Filesystem, `readonly_paths` | **Write denied.** Reading is permitted, writing is refused. |
| Filesystem, `denied_paths` | **Enforced natively.** No host ACL mutation on tier 1. |
| Network, omitted | **Denied.** A DNS lookup failed with the default posture. |
| Network, `IngressHostLoopbackAllow` | Reported as a capability, so loopback is a first-class knob here. |

The measurement method: a throwaway Rust crate depending on `mxc-sdk` 1.0.1,
calling `v1::run` / `v1::spawn_with_pty` / `v1::probe` with minimal
`ContainerRequest`s and observing the outcomes. It is reproducible on any
machine with the SDK; expect different tier results on hosts below 25H2.

Two consequences for the design:

- **The environment allowlist is mandatory, not optional.** A fence that omits
  `process.env` does not inherit the parent's keys, which is the right result, but a fence that
  supplies one without `SYSTEMROOT` and `LOCALAPPDATA` cannot start a Windows
  process at all. Both names must be seeded from the real environment and
  passed through; neither is a secret.
- **No filesystem policy is the strictest policy.** That is the correct default,
  and it means every workload needs an explicit grant before it can read its own
  directory. Budget for that in the authoring loop.

### 6.2 Gaps that apply regardless of tier

- **Learning mode, the deny-report authoring loop, is Windows-only.** MXC
  supports three operating modes: Enforcement (ungranted access is blocked),
  Learning (blocked *and* recorded to a JSON activity report), and Permissive
  (allowed and recorded). Learning is the loop that turns observed denials into
  a least-privilege policy. On Linux and macOS the capability strings are
  ignored: enforcement with no report.
- **Windows filesystem policy varies by tier** (§6). On tier 3 hosts, grants
  *and* denies are enforced by host-path DACLs, which mutate security
  descriptors for the duration of a run; tier 1 requires `processmodel.dll`
  with its OS feature enabled.
- **Concurrent runs sharing a container identifier can revoke each other's
  DACLs.** Parallel subagent shells therefore need distinct identifiers.
- **MSYS2/Cygwin binaries cannot initialize inside any AppContainer-based tier**,
  and the same failure is reported on tier 1 (measured above). AppContainer
  is the Windows sandbox identity MXC derives for the workload; MSYS2 (the
  runtime underneath Git Bash) creates its inter-process signal pipe in a
  global object namespace that AppContainer processes cannot reach, so the
  runtime fails during DLL initialization. This is what makes Git Bash unusable
  in-fence on Windows, and it is why the previous attempt forced commands
  through `cmd`.
- **Tier 1 cannot translate NT paths to drive letters**: the Win32 call
  `GetFinalPathNameByHandleW` with `VOLUME_NAME_DOS` fails with access denied,
  which breaks Node's `realpathSync.native`, Deno under scoped permissions, and
  Vite/Vitest. The apparent workaround, granting the system drive readable,
  defeats an allow-only policy.
- **The "not a security boundary" disclaimer was removed at general
  availability without a published audit.** Backend-specific limits are still
  documented. Prefer the backend docs over the README's silence.
- **No approval gate exists**, so a denied effect must surface as a tool error
  the model can read and report. Silent failure teaches the model to retry a
  different route.

---

## 7. Objections considered

**O1 — "Use a VM for everything; process containers are weak."**
A microVM or WSL container is a stronger boundary, and for genuinely hostile
targets it is the right answer. But it is the wrong default for the dominant
workload: short, model-authored shell commands fired many times in parallel by
subagents. VM tiers cost startup latency, and MXC's own WSL container caps
concurrency at eight, cannot stream stdin during creation-and-run, and offers
all-on/all-off networking. The position is that the *policy primitive* can
select a heavier backend per policy when the risk warrants it; the default must
be the tier that makes containment always-on rather than occasional.

**O2 — "Approval gates are the answer; ask the user."**
`jinn` has no gate today, and the case for adding one as the *primary* control
is weak: per-command dialogs train click-through within a week, and opportunistic
injection exists precisely to exploit the approve reflex. A gate does not bound
the blast radius of an approved-but-wrong command. Gates and containment are
complementary: contain the default, gate the exceptions the policy cannot
express (an unusual path grant, a new MCP server).

**O3 — "Linux first; Bubblewrap exists and Windows MXC is a mess."**
The mess is real (§6.2) but it is a shell-choice problem isolated behind the
runner boundary, not a reason to leave Windows uncontained. The asymmetry runs
the other way: Linux already has decades of containment primitives and this is
parity work, while Windows is where a model-authored `rm -rf` is least impeded
today, and the only platform with the learning-mode authoring loop. Windows
first is not a concession; it is where the work has value.

**O4 — "MXC says it is not a security boundary."**
Correct, and the position does not claim otherwise. The disclaimer's removal at
1.0 without a published audit lowers trust further, and the design assumes the
mechanism can fail: an unavailable backend must degrade to a visible
"uncontained" state, never silently. Blast-radius reduction against model error
and opportunistic injection is worth having under exactly that caveat, which
is why C4 is a claim of the position and not a footnote.

**O5 — "Wait for the enterprise story (Intune, agent identity)."**
Those controls govern how organizations constrain agents; `jinn` is a
power-user harness where the developer-authored policy *is* the feature. Their
absence does not block adoption. The design consequence is only that the
`EffectPolicy` artifact should be stable and externally reviewable, so a future
managed layer can override it rather than replace it.

---

## 8. Related work

- **GitHub Copilot CLI / VS Code agent host** ship MXC-backed shell and local
  MCP sandboxing, generally available since 2026-10. Their documentation states
  first-party file tools honor policy only on a best-effort basis, an industry
  instance of the spawn-site census error this paper warns against in §2.2.
- **OpenAI Codex** sandboxes with Bubblewrap on Linux (with a bundled fallback
  helper for distros that restrict unprivileged user namespaces) and MXC on
  Windows; it has reported the same MSYS2 failure upstream.
- **Bubblewrap / Landlock / seccomp** are the Linux primitives a `BwrapRunner`
  would wrap directly. No learning-mode equivalent exists; policy authoring is
  manual.
- **Windows Sandbox** is a per-run VM: powerful and heavyweight, and a
  non-option for per-command agent use, the trade MXC's lightweight tier
  exists to avoid.
- **rho**, the author's prior harness, confines to a project root in-process:
  sandbox-as-path-convention rather than OS enforcement. It is the contrast
  that motivates this paper: a fence the workload's own author controls is not
  a fence.
- **MXC** itself: MIT-licensed, semver-stable `mxc_sdk::v1` surface since 1.0,
  1.0.1 measured here.

---

## 9. Conclusion

The position, restated:

1. **Policy the primitive.** `EffectPolicy` describes effects; runners and the
   file-tool guard enforce it. A sandbox is one backend, not the boundary.
2. **ToolGuard first.** It closes two of the three kill chains with no
   dependency, on every platform, before any sandbox exists.
3. **MXC for Windows process effects.** Measured, it is strong on its column of
   the coverage table (native enforcement, deny-by-default, a real authoring
   loop), and its gaps are the kind a runner boundary absorbs.
4. **Honesty as a feature.** Uncovered surfaces are withheld or documented;
   denials are visible; the whole story is blast-radius reduction and says so.

The one-sentence version, for every future design argument: **contain the
effects, not the tools; adopt the sandbox, not the sandbox's promise.**

---

## 10. Maintaining this document

The census in §3 is the part that rots. Re-run it whenever a tool is added, a
spawn site is introduced, or a new outbound client appears. The measured table
in §6.1 should be re-verified when `mxc-sdk` crosses a minor version; the tier
table in §6 can change underneath it with a Windows update. The adversary table
in §4 should change rarely; if it changes often, the boundary is being argued
rather than designed.