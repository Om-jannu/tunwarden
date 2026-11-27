# Spike results (M1)

Date: 26 Nov 2026 · Code: `spikes/` (throwaway, not part of the workspace) · Design: `docs/design.md` section 12

| Spike | Status | Ran on |
| --- | --- | --- |
| S1 Windows stop, job objects, `gcloud.cmd` | **Not run**: needs Windows | – |
| S2 Readiness from the socket table | Done | macOS 27.2 arm64 |
| S3 Named pipe restricted to the user | **Not run**: needs Windows | – |
| S4 Crash sweep | Done | macOS 27.2 arm64, Linux arm64 (Docker, Debian) |

## S2: readiness from the OS socket table

Tool: `spikes/src/bin/s2_listeners.rs`. It spawns a command in its own process group, polls `listeners::get_all()` every 200 ms, and checks that the listening socket belongs to the command's process tree (from `sysinfo` parent PIDs). It never connects to the port.

| Case | Socket owner | In tunnel's tree | Ready after |
| --- | --- | --- | --- |
| `custom`: `python3 -m http.server` | the Python process | yes | 0.2–0.6 s |
| `custom` via a wrapper (`sh -c 'python3 ...'`) | the grandchild | yes | 0.2 s |
| `ssh -L` through a Docker OpenSSH server | `ssh` | yes | 0.2 s |
| `kubectl port-forward` to a pod in k3s | `kubectl` | yes | 0.2 s |
| Port already held by an unrelated process | the other process | **no** | – |

Findings:

1. **Ownership works** for ssh, kubectl and wrapped commands. A port held by someone else is correctly *not* counted as ready, so it can be reported as a port conflict. The fallback in design 8.1 ("someone is listening and our process is alive") would have hidden exactly that case. It is dropped: if ownership cannot be read, `doctor` reports it and readiness fails.
2. **New sockets show up on the next poll.** The `listeners` crate does not cache stale results in a long-running process. (One early run looked like a miss; repeating it with output visible showed the listener was found at 0.2 s.)
3. **Cost per probe:** `listeners::get_all()` 1–2 ms, `get_process_by_port()` 12–17 µs, a `sysinfo` process refresh 5–6 ms. One shared snapshot per tick (design 8.1) costs well under 10 ms. At the default 15-second interval that is close to zero CPU.
4. **gcloud IAP binds the local port only after a test connection succeeds.** The SDK source (`googlecloudsdk/command_lib/compute/iap_tunnel.py`, `IapTunnelProxyServerHelper.Run`) runs `_TestConnection()` through IAP first, then opens the local sockets and prints `Listening on port [N].` So for `gcloud-iap`, "port listening" also means IAP accepted a connection at startup, which is a stronger readiness signal than for other types. A real IAP run was not done, because it needs a GCP project (see "Still to do").
5. **gcloud runs as one process.** The `gcloud` script ends with `exec python ... gcloud.py`, so the PID Tunwarden spawns is the PID that owns the socket. (It execs `gocloud` instead only when `CLOUDSDK_USE_GOCLOUD` is set, and that binary is not installed here.)
6. **kubectl confirms the liveness limit (FR-49).** After the pod was deleted, `kubectl port-forward` kept listening. It exited only on the next connection attempt, with `error: lost connection to pod`. Readiness alone cannot catch this.

## S4: crash sweep

Tools: `spikes/src/bin/s4_sweep.rs` and `spikes/run_s4.sh`. A fake daemon spawns a fake tunnel in its own process group (a shell plus a background grandchild), records PID, PGID and start time, and is then killed with `kill -9`. A second run performs the sweep.

| Case | macOS | Linux |
| --- | --- | --- |
| Tunnel processes left after `kill -9` of the daemon | all 3 | all 3 |
| Same, with `PR_SET_PDEATHSIG(SIGKILL)` on the child | – | the leader dies, **the grandchildren stay** |
| Tunnel leader exited earlier, grandchild still running | grandchild stays | grandchild stays |
| Sweep with a wrong start time (simulated PID reuse) | left alone ✔ | left alone ✔ |
| Sweep, leader alive | group gone ✔ | group gone ✔ |
| Sweep, leader gone but group has members | group gone ✔ | group gone ✔ |

Findings:

1. **The sweep is required on both macOS and Linux**, as PRD v2.1 says. Nothing cleans up tunnel processes when the daemon dies.
2. **The parent-death signal does not help and is dropped.** It kills only the direct child, and the grandchildren stay. It also removes the leader whose start time the sweep checks.
3. **The sweep needs a second rule, now in design 6.3.** The first version only checked the leader's PID and start time. It missed groups whose leader had already exited while grandchildren were still running. The rule now has four cases:
   - leader alive, start time matches: kill the group;
   - leader alive, start time differs: the PID was reused, leave it alone;
   - leader gone, group still has members: kill the group. POSIX does not reuse a process ID while a process group with that ID exists, so the group is still ours;
   - leader gone, group empty: nothing to do.
4. **Tagging processes with an environment variable is not possible on macOS.** `sysinfo` and `ps -E` both return an empty environment for other processes, even ones the same user started. So the process group is the only reliable handle.
5. **Process start time has one-second resolution.** A PID reused within the same second as the original start is the only case the guard can miss. That is very unlikely, so it is accepted.
6. **Known gap: a process that leaves its group escapes the sweep, and also escapes a normal stop.** One example is a forked SSH ControlMaster from `ControlPersist` in the user's `~/.ssh/config`. See "Open question" below.

## Extra checks done along the way

These were not planned spikes. They give real error text for the credential patterns (FR-45) and confirm the SSH defaults (FR-42). OpenSSH 10.3 client against an OpenSSH server on Alpine:

| Situation | Output | Classification |
| --- | --- | --- |
| Server stops answering, `ServerAliveInterval=2`, `ServerAliveCountMax=2` | `Timeout, server 127.0.0.1 not responding.` after 6 s | retry (not a credential problem) |
| Unknown host key with `BatchMode=yes` | `Host key verification failed.` | `host key` |
| Wrong key | `root@127.0.0.1: Permission denied (publickey,password,keyboard-interactive).` | `credentials` |
| Local port taken, `ExitOnForwardFailure=yes` | `bind [127.0.0.1]:15437: Address already in use` … `Could not request local forwarding.` | `port conflict` (race after Tunwarden's own pre-check) |
| kubectl unknown context | `Error in configuration: context was not found for specified context: nope` | `invalid config` |
| gcloud with no account (IAP tunnel and `auth print-access-token`) | `You do not currently have an active account selected.` | `auth expired` (needs `tunwarden login`) |

With the default `ServerAliveInterval=15` and `ServerAliveCountMax=3`, a frozen server makes ssh exit after about 45 seconds.

**`gcloud auth print-access-token` must run with stdin set to null.** With stdin attached to a terminal, it hung during this spike. With stdin set to null, it failed in under a second. The design already sets stdin to null for tunnels. Section 9 now says the same for the auth check.

## gcloud against a real environment (26 Nov 2026)

Run on macOS against a company UAT project with gcloud 591. Names are replaced by placeholders (`uat-project`, `uat-vm`, `user@example.com`). Nothing was sent through any tunnel, and nothing in the project was changed.

**Expired session.** The account's org session had expired, though the user believed they were logged in:

| Command | Result |
| --- | --- |
| `gcloud auth print-access-token` (stdin and stdout null) | **exit 0** in 1 s |
| `gcloud compute instances describe uat-vm ...` | exit 1: `There was a problem refreshing your current auth tokens: Reauthentication failed. cannot prompt during non-interactive execution.` |
| `gcloud compute start-iap-tunnel uat-vm ...` | exit 1 with the same text, before any local port opened |

Findings:

1. **`print-access-token` is not a valid auth check.** It reported success while every real call needed re-authentication, most likely because it returns a cached access token. That token can stay valid for up to an hour, so FR-22 would miss an expired login for that long.
2. **The real expired-session text** is `Reauthentication failed. cannot prompt during non-interactive execution.` It is the pattern for `auth expired` (FR-24).

**Candidate replacement check**, run after the user logged in again:

| Command | Result |
| --- | --- |
| `gcloud config config-helper --force-auth-refresh --format="value(credential.token_expiry)"` | exit 0 in 1 s; prints only the new token's expiry |
| `gcloud projects describe uat-project --format="value(projectNumber)"` | exit 1: the account has no permission to read the project. Unusable as a general check, because it depends on IAM, not on login state |

`config-helper --force-auth-refresh` forces a real refresh, so it should fail when the session has expired. It was not run during the expired session, so this is **not confirmed yet**. Next time the session expires, run it before logging in.

**IAP tunnel after logging in again:**

| Command | Result |
| --- | --- |
| `gcloud compute instances describe uat-vm --format="value(status)"` | `RUNNING` |
| `gcloud compute start-iap-tunnel uat-vm PORT --local-host-port=127.0.0.1:18099 ...` | printed `Testing if tunnel connection works.`, then exit 1: `While checking if a connection can be made: Error while connecting [4033: 'not authorized'].` No local port opened, no processes left behind |

Findings:

3. **IAP error 4033 means the connection is not allowed.** It needs its own pattern and the reason `credentials`, with a hint about IAM (`iap.tunnelInstances.accessViaIAP`). It must not be classified as `auth expired`, because logging in again doesn't fix it, and it must not be retried.
4. **gcloud confirmed it binds only after the IAP test connection succeeds** (S2 finding 4). A failed test means the port never opens, so readiness never passes.
5. gcloud prints a `WARNING` suggesting NumPy for better tunnel bandwidth. It is harmless and must not match any error pattern.

The 4033 above came from a wrong remote port in the test command, not from missing access. It still shows that IAP reports a disallowed port with the same 4033 error as missing IAM permission, so the hint for 4033 should mention both.

**IAP tunnel with the correct port:**

| Check | Result |
| --- | --- |
| Readiness (s2_listeners, never connecting) | ready after **1.5 s**; socket owned by the spawned PID (the Python process, via the launcher's `exec`) |
| Stop with SIGTERM to the process group | no processes left, port closed |
| Stop with `kill -9` to the process group | one process in the group; none left, port closed |

6. **gcloud IAP runs as a single process** in this setup, so stopping it is simple on macOS. The process group still matters for `custom` wrappers.

## Still to do

- **S1 and S3 need a Windows machine.** They will run on the developer's Windows laptop, later in M1, before the Windows parts of `process` and `daemon` are written.
- **Confirm `config-helper --force-auth-refresh` fails during an expired session.**

## Decided after the spikes

SSH tunnels turn off connection sharing by default (`-o ControlMaster=no -o ControlPath=none`). The reasoning, kept for reference: If a user's `~/.ssh/config` sets `ControlMaster auto` with `ControlPersist`, a tunnel's ssh may either attach to a shared connection it does not own, or fork a background master that outlives a stop. Turning sharing off makes every tunnel own its own connection, at the cost of a fresh SSH handshake on each start. This has not been tested yet.
