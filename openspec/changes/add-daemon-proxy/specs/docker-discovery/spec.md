# Spec Delta

## Purpose

Defines how running Docker containers become routes: which workspace a container belongs to, what it is called, which port it is reached on, and the `cyamus.*` label contract that overrides each choice.

## ADDED Requirements

### Requirement: Docker endpoint
The daemon SHALL connect to Docker using `DOCKER_HOST` if set. Otherwise it SHALL use the endpoint of the current Docker CLI context, and fall back to `/var/run/docker.sock`. Only Unix-socket endpoints are required to work. While Docker is unreachable, the daemon SHALL retry at most every 5 seconds and serve no container routes.

#### Scenario: OrbStack or Colima context
- **WHEN** the current Docker context's endpoint is `unix:///Users/me/.orbstack/run/docker.sock` and `DOCKER_HOST` is unset
- **THEN** the daemon discovers containers from that socket

### Requirement: Workspace membership
A running container SHALL belong to a registered workspace when either:
- it has labels `cyamus.project` and `cyamus.workspace` (a branch label) that match a registered workspace; or
- it has the compose label `com.docker.compose.project.working_dir` and that directory, canonicalized, is a registered worktree root or lies inside one.

Explicit labels SHALL take precedence. Matching by path SHALL pick the longest matching worktree path. A container matching no workspace SHALL NOT be routed.

#### Scenario: Compose file copied into the worktree
- **WHEN** `docker compose up` runs in worktree `/w/feat`, which is registered as `feat-x` in project `myproj`, from a compose file with no `cyamus.*` labels
- **THEN** its containers belong to workspace `feat-x` of `myproj`

#### Scenario: Compose run from a subdirectory of the worktree
- **WHEN** the compose `working_dir` is `/w/feat/infra`
- **THEN** the containers belong to the workspace registered at `/w/feat`

#### Scenario: Explicit labels for a non-compose container
- **WHEN** a container started with `docker run` has `cyamus.project=myproj` and `cyamus.workspace=feat-x`
- **THEN** it belongs to that workspace

#### Scenario: Compose project outside any worktree
- **WHEN** the compose `working_dir` is not inside any registered worktree
- **THEN** the container is not routed

### Requirement: Opt-out
A container labeled `cyamus.enable=false` SHALL NOT be routed. All other containers that belong to a workspace and have a usable port SHALL be routed by default.

#### Scenario: Database opted out
- **WHEN** a workspace's `db` service has `cyamus.enable: "false"`
- **THEN** no route exists for `db.<branch-label>.<project>.localhost`

### Requirement: Service name
A route's service name SHALL be the `cyamus.service` label if present. Otherwise it SHALL be the compose service name (`com.docker.compose.service`), and otherwise the container name. The result SHALL be lowercased and every run of characters outside `[a-z0-9]` replaced with `-`. A container whose resulting name is empty or longer than 63 characters SHALL NOT be routed.

#### Scenario: Compose service name used
- **WHEN** compose service `web` belongs to workspace `feat-x` of `myproj`
- **THEN** the route is `web.feat-x.myproj.localhost`

#### Scenario: Renamed with a label
- **WHEN** compose service `api` has `cyamus.service: backend`
- **THEN** the route is `backend.feat-x.myproj.localhost`

### Requirement: Target port selection
The target SHALL be a published host port of the container. If `cyamus.port` is set, the target SHALL be the host port published for that TCP container port. If it is not set and the container publishes exactly one TCP *container* port, that port SHALL be used. Bindings of the same container port on several host addresses (e.g. `0.0.0.0` and `::`) count as one port. Otherwise the container SHALL NOT be routed. The target address SHALL be `127.0.0.1` when the port is bound on `0.0.0.0` or `127.0.0.1`, otherwise `::1` when bound on `::` or `::1`, otherwise the bound host IP. Container-network IPs SHALL NOT be used.

#### Scenario: Ephemeral published port
- **WHEN** compose service `web` has `ports: ["3000"]` and Docker publishes it on host port 49321
- **THEN** the route targets `127.0.0.1:49321`

#### Scenario: Port published on IPv4 and IPv6
- **WHEN** Docker publishes container port 3000 as both `0.0.0.0:32768` and `[::]:32768`
- **THEN** the container counts as publishing one port, and the route targets `127.0.0.1:32768`

#### Scenario: Several published ports with a label
- **WHEN** a container publishes 3000 and 9229, and is labeled `cyamus.port: "3000"`
- **THEN** the route targets the host port published for container port 3000

#### Scenario: Several published ports without a label
- **WHEN** a container publishes 3000 and 9229 and has no `cyamus.port` label
- **THEN** it is not routed, and status reports it as ambiguous, listing both ports

#### Scenario: No published ports
- **WHEN** a container belongs to a workspace but publishes no ports
- **THEN** it is not routed, and status reports that it has no published port

### Requirement: Duplicate hostnames
If two routable containers produce the same hostname, the container created first SHALL get the route, with the container name breaking ties. The other SHALL be reported by status as a duplicate, naming the winning container.

#### Scenario: Two replicas of a service
- **WHEN** compose runs `web` with 2 replicas, each publishing an ephemeral port
- **THEN** one replica is routed, and status reports the other as a duplicate

### Requirement: Invalid labels are reported, not fatal
A container with an invalid `cyamus.*` label value SHALL NOT be routed, and status SHALL report the label and the reason. Invalid values include a non-numeric `cyamus.port`, a `cyamus.port` that isn't published, `cyamus.enable` other than `true` or `false`, or only one of `cyamus.project` and `cyamus.workspace`. Other containers SHALL be unaffected.

#### Scenario: Port label not published
- **WHEN** a container is labeled `cyamus.port: "8080"` but publishes only 3000
- **THEN** it is not routed, and status reports that container port 8080 is not published
