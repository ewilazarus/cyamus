# Spec Delta

## MODIFIED Requirements

### Requirement: Hostname scheme
A workspace route's hostname SHALL be `<service>.<branch-label>.<project>.localhost`. A shared-stack route's hostname SHALL be `<service>.<project>.localhost`. The two shapes have different label counts, so they cannot collide. Matching SHALL be case-insensitive and SHALL ignore the port in the `Host` header and any trailing dot.

#### Scenario: Request for a routed service
- **WHEN** a request arrives with `Host: Web.Feat-X.myproj.localhost:1355`
- **THEN** it is matched to the route `web.feat-x.myproj.localhost`

#### Scenario: Request for a shared service
- **WHEN** a request arrives with `Host: mailpit.myproj.localhost`
- **THEN** it is matched to the shared-stack route `mailpit.myproj.localhost`
