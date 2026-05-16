# SIP3 Harbor Registry Deployment Design

## Goal

Move production deployment off the target server build path. CI should build SIP3 images, push them to a self-hosted public Docker registry, and production servers should only pull pinned image tags and restart containers.

## Confirmed decisions

- Registry: self-hosted Harbor on a separate server with its own public domain.
- Access: the registry is publicly reachable and requires login; anonymous pulls are not assumed.
- Scope: the registry should be reusable by other projects, not only SIP3.
- Deployment target: production must not compile Rust or frontend assets on the application server.
- Testing: code-level tests stay in CI; production only performs image pull, container start, and health checks.
- Versioning: production must use immutable image tags, not `latest`.

## Problem statement

The current production flow runs `docker compose up -d --build` on the application server. That makes deployment slow, couples release time to the machine's CPU and memory, and creates avoidable risk when build workloads contend with production workloads.

The goal is not only to shorten deploys. It is to remove compile-time variability from production and make releases reproducible, predictable, and rollback-friendly.

## Recommended architecture

Use a three-stage release pipeline:

1. **CI validation**
   - run backend format/build/test/clippy
   - run frontend install/build
   - fail fast on code or dependency issues

2. **CI image build and publish**
   - build backend and frontend images from the repo
   - tag each image with an immutable identifier such as git SHA and release tag
   - push images to Harbor over HTTPS with authenticated credentials

3. **Production rollout**
   - update the compose file or deployment manifest to the new immutable tag
   - run `docker compose pull`
   - run `docker compose up -d`
   - verify health endpoints

## Component responsibilities

| Component | Responsibility |
| --- | --- |
| Harbor server | Stores versioned images, handles authentication, supports cleanup and reuse across projects. |
| CI workflow | Runs tests and builds images; publishes only validated artifacts. |
| Production host | Pulls and runs already-built images; never performs source compilation. |
| Deployment config | Pins exact image tags and registry paths for backend/frontend images. |

## Workflow details

### CI

The current CI already proves the source tree with backend and frontend builds. That should remain the gate for correctness.

After tests pass, CI should build Docker images and push them to Harbor. Recommended tags:

- `git-<shortsha>` for immutable deployment
- `vX.Y.Z` for releases
- optionally `main` for convenience only, never as the production pin

### Production

Production should switch from local `build:` instructions to `image:` references, for example:

```yaml
services:
  backend:
    image: harbor.air32.cn/sip3/backend:git-abc1234
  frontend:
    image: harbor.air32.cn/sip3/frontend:git-abc1234
```

The production update command becomes:

```bash
docker compose pull
docker compose up -d
curl -f http://127.0.0.1:3000/api/health
```

This keeps the deployment path simple and removes build-time memory pressure from the application server.

## Harbor setup

Harbor should be deployed on a dedicated machine with:

- TLS certificate for the registry domain
- authenticated user/project access
- storage volume sized for retained images
- retention and garbage-collection policy
- optional robot accounts for CI push and production pull

Recommended access model:

- CI uses a push-only robot account
- production uses a pull-only account
- human admin access is separate

## Image layout

Keep images separated by concern:

- `.../sip3/backend`
- `.../sip3/frontend`

Do not bundle everything into a single monolithic image unless a later operational constraint forces that choice. Separate images keep release/rollback scope smaller and match the existing container topology.

## Rollback model

Rollback should be a tag change only:

1. record the currently deployed tag
2. switch compose back to the previous known-good tag
3. `docker compose pull && docker compose up -d`

Because tags are immutable, rollback does not depend on rebuilding or on registry-side state changes.

## Error handling and operational safeguards

- If image pull fails, the production host must keep the previous running containers until the new pull succeeds.
- If Harbor is unavailable, CI should fail the publish step rather than marking the release complete.
- If cleanup removes an image still referenced by production, the deploy process should fail loudly during pull; production tags must therefore be retained by policy.
- Registry credentials must never be stored in tracked compose files.

## Migration plan

1. Stand up Harbor on the registry server and confirm HTTPS login.
2. Add CI steps to build and push backend/frontend images.
3. Change production compose files from `build:` to `image:` references.
4. Pin production to release tags.
5. Update deployment docs to describe pull-based rollout.
6. Remove any remaining `docker compose up -d --build` instructions from production docs.

## Testing strategy

- Keep existing backend and frontend CI checks.
- Add a Docker publish step in CI that runs only after source checks pass.
- Verify a deployed host can:
  - authenticate to Harbor,
  - pull the pinned tags,
  - start containers without local builds,
  - pass health checks.
- Add a rollback drill using the previous tag.

## Out of scope

- Rewriting application code for deployment.
- Changing SIP runtime behavior.
- Moving production databases into the registry flow.
- Building a custom image distribution system instead of Harbor.

## Open questions

- Whether to keep per-branch preview tags in addition to release tags.
- Whether production hosts should use a compose override file or an environment variable to select the image tag.
- Whether Harbor cleanup should be time-based only or also protect explicitly pinned release tags.

