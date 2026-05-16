# GHCR to Harbor Sync Design

## Problem

The current Docker publish path depends on GitHub-hosted runners pushing directly to `harbor.air32.cn`. That path is unreliable across the current network boundary, so image publish becomes the release bottleneck even when backend and frontend validation pass.

The new design must keep GitHub CI usable, avoid direct GitHub-to-Harbor pushes, and preserve Harbor as the only runtime registry used by production.

## Goals

- Keep backend and frontend validation in GitHub Actions.
- Publish release images to a registry that GitHub can reach reliably.
- Sync images into `harbor.air32.cn` from the Harbor host itself.
- Keep production deployment pulling only from Harbor.
- Keep rollout and rollback explicit by tag.

## Non-Goals

- No direct auto-deploy from GitHub to production.
- No dependency on Harbor built-in replication as the primary path.
- No requirement for Harbor to maintain stable outbound access to GHCR without retries or local operator control.

## Chosen Approach

Use GitHub Actions to publish container images to `ghcr.io`, then run a host-local sync job on the Harbor server that copies selected tags from GHCR into Harbor with `skopeo`.

This is preferred over direct Harbor pushes because:

- GitHub can reach GHCR reliably.
- The domestic leg is reduced to Harbor-host-local pulls from GHCR.
- `skopeo copy` is better suited than `docker pull/tag/push` for registry-to-registry sync, retries, and scripting.
- Harbor remains the only production-facing image source.

## Alternatives Considered

### 1. Harbor host `docker pull/tag/push`

This works, but it depends on the local Docker daemon for sync operations, writes more local layers, and is less clean for registry mirroring than `skopeo`.

### 2. Harbor built-in replication plus proxy

This was rejected as the default path because the root problem is unstable Harbor-host access to GHCR. Adding replication and proxy policy would increase operational complexity without improving the core network constraint enough.

## Architecture

### CI publish path

On pushes to `main` or `master`, and on release tags, GitHub Actions will:

1. Run backend validation.
2. Run frontend validation.
3. Build backend and frontend container images.
4. Push them to:
   - `ghcr.io/wendal/sip3/backend`
   - `ghcr.io/wendal/sip3/frontend`

Tags:

- Commit tags: `git-<shortsha>`
- Release tags: upstream git tag names such as `v1.3.0`

### Harbor sync path

The Harbor host will run a script such as `/opt/sip3/scripts/sync-from-ghcr.sh` that:

1. Accepts an image tag, for example `git-9700c58`.
2. Copies backend and frontend images from GHCR into Harbor:
   - `ghcr.io/wendal/sip3/backend:<tag>` -> `harbor.air32.cn/sip3/backend:<tag>`
   - `ghcr.io/wendal/sip3/frontend:<tag>` -> `harbor.air32.cn/sip3/frontend:<tag>`
3. Verifies the Harbor tags exist after sync.
4. Exits non-zero if either copy fails.

The script may later be wrapped in a `systemd` timer or cron job, but manual tag-based sync is the initial operating mode.

### Production deploy path

Production continues to deploy from Harbor only:

1. Set `IMAGE_TAG` in `.env`.
2. Run `docker compose pull`.
3. Run `docker compose up -d`.
4. Validate with container health and `/api/health`.

## Repository Changes

### `.github/workflows/ci.yml`

- Remove direct Harbor login and push steps.
- Replace them with GHCR login and push steps.
- Keep backend and frontend validation unchanged.
- Keep Docker publish limited to push events on main/master and release tags.

Authentication:

- Prefer `GITHUB_TOKEN` with `packages: write` permission for pushes to the repository-owned GHCR namespace.
- If repository policy requires it, use a dedicated fine-grained token instead.

### `docs/deployment.md`

Update the deployment guide so the default release path is:

`GitHub Actions -> GHCR -> Harbor host skopeo sync -> production pull from Harbor`

Document:

- manual sync of a chosen tag
- post-sync verification
- production deploy by `IMAGE_TAG`
- rollback to a previous Harbor tag

## Harbor Host Changes

### Sync script

Add a host-local script that:

- takes one required tag argument
- copies backend and frontend with `skopeo copy --all`
- supports public GHCR reads with no auth by default
- can optionally accept future auth env vars if GHCR visibility changes

Recommended command shape:

```bash
skopeo copy --all docker://ghcr.io/wendal/sip3/backend:${TAG} docker://harbor.air32.cn/sip3/backend:${TAG}
skopeo copy --all docker://ghcr.io/wendal/sip3/frontend:${TAG} docker://harbor.air32.cn/sip3/frontend:${TAG}
```

### Optional scheduler

The first version should keep sync manual and tag-driven. A timer may be added later for convenience, but automatic sync is not required for correctness.

## Error Handling

### CI failure

If GHCR publish fails, the workflow fails and no new tag should be considered releasable. Harbor and production remain unchanged.

### Harbor sync failure

If the Harbor host cannot reach GHCR or one image copy fails, the script exits non-zero and leaves production unchanged.

### Deploy failure

If production deploy fails after a successful Harbor sync, operators can revert `IMAGE_TAG` to the previous Harbor tag and rerun the compose pull and up sequence.

## Verification

### CI verification

- Confirm backend and frontend validation pass.
- Confirm GHCR contains both images for the expected tag.

### Sync verification

- Confirm Harbor contains both copied tags after the sync script runs.
- Verify with `skopeo inspect` or `docker manifest inspect`.

### Production verification

- `docker compose ps`
- `curl -f http://127.0.0.1:3000/api/health`

## Operational Defaults

- GitHub `main` pushes automatically publish images to GHCR.
- Harbor sync is manual by explicit tag in the first version.
- Production deployment remains manual by explicit `IMAGE_TAG`.

This preserves controlled releases while removing the unreliable GitHub-to-Harbor leg.
