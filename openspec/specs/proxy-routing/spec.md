# proxy-routing Specification

## Purpose
Defines the hostname scheme the daemon serves and how it forwards HTTP and WebSocket traffic to the services behind those hostnames.

## Requirements

### Requirement: Hostname scheme
A workspace route's hostname SHALL be `<service>.<branch-label>.<project>.localhost`. A shared-stack route's hostname SHALL be `<service>.<project>.localhost`. The two shapes have different label counts, so they cannot collide. Matching SHALL be case-insensitive and SHALL ignore the port in the `Host` header and any trailing dot.

#### Scenario: Request for a routed service
- **WHEN** a request arrives with `Host: Web.Feat-X.myproj.localhost:1355`
- **THEN** it is matched to the route `web.feat-x.myproj.localhost`

#### Scenario: Request for a shared service
- **WHEN** a request arrives with `Host: mailpit.myproj.localhost`
- **THEN** it is matched to the shared-stack route `mailpit.myproj.localhost`

### Requirement: Request forwarding
A request for a routed hostname SHALL be forwarded to the route's target over HTTP/1.1. The method, path, query, body and headers SHALL be preserved, except for hop-by-hop headers. The original `Host` header SHALL be preserved. `X-Forwarded-Host`, `X-Forwarded-Proto: http` and `X-Forwarded-For` SHALL be set. The response SHALL be streamed back unchanged.

#### Scenario: Plain request
- **WHEN** `GET /api/items?x=1` is sent to `web.feat-x.myproj.localhost:1355`, routed to `127.0.0.1:49321`
- **THEN** the backend receives `GET /api/items?x=1` with `Host: web.feat-x.myproj.localhost:1355` and the `X-Forwarded-*` headers, and the client receives the backend's response

#### Scenario: Streaming response
- **WHEN** the backend responds with server-sent events
- **THEN** each event reaches the client as the backend sends it, not after the response completes

### Requirement: WebSocket passthrough
An HTTP/1.1 upgrade request to a routed hostname SHALL be forwarded with its upgrade headers. If the backend accepts the upgrade, the daemon SHALL relay bytes in both directions until either side closes.

#### Scenario: Dev server hot reload
- **WHEN** a browser opens a WebSocket to `ws://web.feat-x.myproj.localhost:1355/` and the backend accepts it
- **THEN** messages flow in both directions until one side closes

### Requirement: Unknown hostname
A request whose hostname matches no route SHALL get a `404` response. The body SHALL name the requested host and list the current routes, as HTML for browsers (`Accept: text/html`) and plain text otherwise.

#### Scenario: Typo in hostname
- **WHEN** a browser requests `http://wbe.feat-x.myproj.localhost:1355/`
- **THEN** it gets a 404 page naming `wbe.feat-x.myproj.localhost` and listing the available routes

### Requirement: Backend unavailable
When a route's target refuses the connection or fails before sending a response, the daemon SHALL respond `502 Bad Gateway` with a body naming the hostname and the target address.

#### Scenario: Container still starting
- **WHEN** a route exists, but nothing accepts connections on its target port yet
- **THEN** the client gets a 502 naming the route and target

### Requirement: Generated URLs follow the request
Links on the route page and the 404 page SHALL use the port of the incoming request's `Host` header. A request without a port, such as one that arrived through the port-80 relay, SHALL get links without a port.

#### Scenario: Browsing through the redirect
- **WHEN** a browser opens `http://cyamus.localhost/`
- **THEN** route links have the form `http://web.feat-x.myproj.localhost/`

#### Scenario: Browsing on the daemon port
- **WHEN** a browser opens `http://cyamus.localhost:1355/`
- **THEN** route links have the form `http://web.feat-x.myproj.localhost:1355/`

### Requirement: Route page
Requests for `cyamus.localhost` SHALL be answered by the daemon itself. `/` SHALL show the route table and unrouted containers as HTML. `/api/status` SHALL return the same information, plus the daemon PID, version and port, as JSON. `cyamus daemon status` uses this endpoint.

#### Scenario: Browse the route table
- **WHEN** a browser opens `http://cyamus.localhost:1355/`
- **THEN** it sees every current route as a clickable link and every unrouted container with its reason

### Requirement: Route updates without restart
Changes to the workspace registry or to running containers SHALL be reflected in routing without restarting the daemon. Registry changes SHALL take effect for the next request after the change. Container changes SHALL take effect within 2 seconds of the corresponding Docker event.

#### Scenario: New workspace
- **WHEN** setup registers a new workspace and its compose stack starts
- **THEN** requests to its service hostnames are routed without restarting the daemon

#### Scenario: Container stopped
- **WHEN** a routed container stops
- **THEN** within 2 seconds its hostname returns 404
