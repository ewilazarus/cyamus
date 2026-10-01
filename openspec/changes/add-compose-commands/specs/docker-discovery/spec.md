# Spec Delta

## MODIFIED Requirements

### Requirement: Workspace membership
A running container SHALL belong to a registered workspace when either:
- it has labels `cyamus.project` and `cyamus.workspace` (a branch label) that match a registered workspace; or
- it has the compose label `com.docker.compose.project.working_dir` and that directory, canonicalized, is a registered worktree root or lies inside one.

Explicit labels SHALL take precedence. Matching by path SHALL pick the longest matching worktree path. A container that matches neither a workspace nor a project's shared stack (see "Shared stack membership") SHALL NOT be routed.

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
- **WHEN** the compose `working_dir` is neither inside a registered worktree nor a project config directory
- **THEN** the container is not routed

## ADDED Requirements

### Requirement: Shared stack membership
A running container without `cyamus.workspace` SHALL belong to the shared stack of project `<p>` when its canonicalized compose `working_dir` equals the canonical path of `$XDG_CONFIG_HOME/cyamus/projects/<p>`. It SHALL then be routed as `<service>.<p>.localhost`. The opt-out, service name, target port, duplicate and invalid-label rules that apply to workspace containers SHALL apply as well. Shared-stack routing SHALL NOT require any workspace of the project to be registered.

#### Scenario: Shared stack started by cyamus compose-shared
- **WHEN** `cyamus compose-shared up -d` starts service `mailpit` for project `myproj` and publishes container port 8025 on host port 49200
- **THEN** `mailpit.myproj.localhost` routes to `127.0.0.1:49200`

#### Scenario: Shared database opted out
- **WHEN** the shared `db` service is labelled `cyamus.enable: "false"`
- **THEN** no route exists for it, and status reports it as opted out
