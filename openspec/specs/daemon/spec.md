# daemon Specification

## Purpose
Defines the machine-wide background proxy process: where it listens, how the CLI starts, inspects and stops it, and how a single up-to-date instance is guaranteed.

## Requirements

### Requirement: Loopback-only listener
The daemon SHALL accept HTTP connections only on `127.0.0.1` and `::1`, on the port given by `CYAMUS_DAEMON_PORT`, defaulting to `1355`. It SHALL NOT listen on any other address. If either address cannot be bound, it SHALL keep serving on the other and log a warning. If neither can be bound, it SHALL exit non-zero.

#### Scenario: Default port
- **WHEN** the daemon starts with `CYAMUS_DAEMON_PORT` unset
- **THEN** it accepts connections on `127.0.0.1:1355` and `[::1]:1355`

#### Scenario: Not reachable from the network
- **WHEN** another machine connects to this machine's LAN address on the daemon port
- **THEN** the connection is refused

#### Scenario: Port in use
- **WHEN** another process already holds port 1355 on both loopback addresses
- **THEN** `cyamus daemon run` exits non-zero with an error naming the port and `CYAMUS_DAEMON_PORT`

### Requirement: Single instance
At most one daemon SHALL run per state directory. The running daemon SHALL hold an exclusive lock on `$XDG_STATE_HOME/cyamus/daemon.lock` and record its PID in `daemon.pid`. A second `cyamus daemon run` SHALL exit non-zero and report the running daemon's PID.

#### Scenario: Second instance
- **WHEN** a daemon is running and `cyamus daemon run` is invoked again
- **THEN** the new process exits non-zero and reports that a daemon is already running, with its PID

### Requirement: Foreground run
`cyamus daemon run` SHALL run the daemon in the foreground, log to stderr, and shut down cleanly on SIGINT or SIGTERM, releasing its lock and removing `daemon.pid`.

#### Scenario: Ctrl-C
- **WHEN** a foreground daemon receives SIGINT
- **THEN** it stops accepting connections, exits 0, and `daemon.pid` no longer exists

### Requirement: On-demand start from setup
`cyamus workspace setup` SHALL ensure a daemon is running, unless `CYAMUS_DAEMON=off` is set. If none is running, setup SHALL start one in the background, detached from setup's process group and terminal, with output appended to `$XDG_STATE_HOME/cyamus/daemon.log`. If starting fails, setup SHALL print a warning and continue. Concurrent setups SHALL start at most one daemon.

#### Scenario: First setup starts the daemon
- **WHEN** no daemon is running and setup runs
- **THEN** a daemon is running after setup returns, and it keeps running after setup's terminal closes

#### Scenario: Daemon disabled
- **WHEN** setup runs with `CYAMUS_DAEMON=off`
- **THEN** no daemon is started

#### Scenario: Daemon fails to start
- **WHEN** the daemon port is held by another program
- **THEN** setup prints a warning that points to `daemon.log`, runs its hooks, and exits 0 if nothing else failed

#### Scenario: Parallel setups
- **WHEN** three setups run at once with no daemon running
- **THEN** exactly one daemon process is running afterwards

### Requirement: Version handover
When setup finds a running daemon whose version differs from its own, it SHALL stop that daemon and start a new one.

#### Scenario: After upgrading cyamus
- **WHEN** a daemon from cyamus 0.2.0 is running and setup from cyamus 0.3.0 runs
- **THEN** afterwards the running daemon reports version 0.3.0

### Requirement: Status command
`cyamus daemon status` SHALL report:
- whether a daemon is running, with its PID, version and listen port;
- whether Docker is reachable;
- every current route: hostname, target, and source container or workspace;
- every container that was discovered but not routed, with the reason.

It SHALL exit 0 when a daemon is running and 1 when none is.

#### Scenario: Running with routes
- **WHEN** a daemon is running and one compose service is routed
- **THEN** status prints the PID, version, port, Docker reachability and the route `web.feat-x.myproj.localhost → 127.0.0.1:49321`, and exits 0

#### Scenario: Not running
- **WHEN** no daemon is running
- **THEN** status prints that the daemon is not running and exits 1

### Requirement: Stop command
`cyamus daemon stop` SHALL send SIGTERM to the running daemon and wait for it to exit. If no daemon is running, it SHALL print a notice and exit 0.

#### Scenario: Stop a running daemon
- **WHEN** a daemon is running and `cyamus daemon stop` is run
- **THEN** the daemon process has exited when the command returns, and the daemon port is free

### Requirement: Port 80 relay install
`cyamus daemon install` SHALL install a relay service that listens on `127.0.0.1:80` and `[::1]:80` only, and forwards each connection unchanged to the daemon port on the same loopback address.
- **Binary:** a root-owned copy of the running `cyamus` binary at `/usr/local/libexec/cyamus-relay`.
- **macOS:** the LaunchDaemon `dev.cyamus.relay` binds as root, then serves from a child process running as `nobody`, with no root identity left (no uid or gid 0, no group 0).
- **Linux:** the systemd unit `cyamus-relay.service` runs as an ephemeral unprivileged user (`DynamicUser=yes`), with only `CAP_NET_BIND_SERVICE`.
- **Lifetime:** the relay SHALL start at boot and be restarted if it exits.
- **Running it:** the command SHALL run privileged steps through `sudo`, printing each command first, and SHALL NOT need to be run as root itself. Afterwards it SHALL verify that `http://cyamus.localhost/` reaches the daemon, and SHALL exit non-zero if it doesn't. `--dry-run` SHALL print the files and commands and change nothing. Setup SHALL NEVER perform this step.

#### Scenario: Install on macOS
- **WHEN** a user runs `cyamus daemon install` with the daemon on port 1355
- **THEN** `dev.cyamus.relay` is loaded, the process accepting connections on port 80 runs as `nobody`, and `http://cyamus.localhost/` answers

#### Scenario: Install on Linux
- **WHEN** a user runs `cyamus daemon install` on Linux with systemd
- **THEN** `cyamus-relay.service` is enabled and running as an unprivileged dynamic user, and `http://cyamus.localhost/` answers

#### Scenario: Dry run
- **WHEN** a user runs `cyamus daemon install --dry-run`
- **THEN** the file contents and commands are printed, and nothing is written or run

#### Scenario: Not reachable from the network
- **WHEN** another machine connects to this machine's LAN address on port 80
- **THEN** the connection is refused, because the relay listens on loopback only

#### Scenario: Run under sudo
- **WHEN** a user runs `sudo cyamus daemon install` and no daemon is running
- **THEN** the relay is installed, no daemon is started as root, and the command explains that the redirect will work once the user's daemon runs

#### Scenario: Missing systemctl on Linux
- **WHEN** `systemctl` is not available
- **THEN** install exits non-zero, names the missing tool, and changes nothing

### Requirement: Relay forwarding
`cyamus daemon relay --to <port> [--listen <port>]` SHALL accept TCP connections on `127.0.0.1` and `::1` at the listen port (default 80). It SHALL forward the bytes of each connection unchanged in both directions to the same loopback address at `--to`. If the target refuses the connection, it SHALL close the client connection. It SHALL NOT interpret HTTP. When started as root, it SHALL bind its sockets and then serve only from a child process running as `nobody`. A relay serving inherited sockets SHALL refuse to start while its real or effective uid or gid is 0, or group 0 is in its group list.

#### Scenario: Forwarding on a high port
- **WHEN** `cyamus daemon relay --listen 18080 --to 1355` runs and a client requests `http://cyamus.localhost:18080/api/status`
- **THEN** the daemon on 1355 answers, with the client's original `Host` header

#### Scenario: Privilege drop verified
- **WHEN** the serving relay starts with group 0 still in its effective gid or its group list
- **THEN** it exits with an error naming its ids, and serves nothing

#### Scenario: Daemon not running
- **WHEN** the relay is running but nothing listens on the target port
- **THEN** client connections are closed without a response, and the relay keeps running

### Requirement: Port 80 relay uninstall
`cyamus daemon uninstall` SHALL stop and remove the relay service, its unit or plist, and `/usr/local/libexec/cyamus-relay`, through `sudo`. Running it when nothing is installed SHALL succeed.

#### Scenario: Uninstall
- **WHEN** the relay is installed and a user runs `cyamus daemon uninstall`
- **THEN** port 80 no longer reaches the daemon, the daemon still answers on 1355, and the installed files are gone

### Requirement: Redirect detection
The CLI SHALL consider port 80 redirected when `GET /api/status` sent to `127.0.0.1:80` with `Host: cyamus.localhost` is answered by the daemon with the daemon's own PID. `cyamus daemon status` SHALL report whether port 80 is redirected. When it is, status SHALL print route URLs without a port, and otherwise with `:<port>`.

#### Scenario: Status with the redirect
- **WHEN** the relay is installed and the daemon is running
- **THEN** status reports that port 80 is redirected, and lists `http://web.feat-x.myproj.localhost/`

#### Scenario: Status without the redirect
- **WHEN** nothing listens on port 80
- **THEN** status reports that port 80 is not redirected, suggests `cyamus daemon install`, and lists `http://web.feat-x.myproj.localhost:1355/`

### Requirement: Lifetime
The daemon SHALL keep running until it is stopped or receives a termination signal. It SHALL NOT exit because no workspaces are registered or because Docker is unreachable.

#### Scenario: Last workspace torn down
- **WHEN** the last registered workspace is torn down
- **THEN** the daemon is still running

#### Scenario: Docker not running
- **WHEN** the daemon starts while Docker is stopped, and Docker starts later
- **THEN** the daemon keeps running, reconnects, and routes containers once Docker is available
