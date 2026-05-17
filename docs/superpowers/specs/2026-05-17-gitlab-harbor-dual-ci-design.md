# GitLab + Harbor Dual CI Design

## Problem

Current deployment reliability for the China-based production server is sensitive to cross-border network reachability. The project already uses GitHub Actions + GHCR, but production should have a fully domestic build-and-deploy path using self-hosted GitLab Runner and Harbor.

## Goal

Keep both pipelines active:

1. GitHub Actions remains available for overseas CI/build/publish.
2. GitLab CI is added for domestic CI/build/publish to Harbor.
3. Production server deploys from Harbor only (`harbor_pull_only`).

## Scope

### In scope

- Add GitLab CI pipeline file for backend/frontend checks and image publish.
- Keep GitHub Actions workflow unchanged except optional documentation alignment.
- Standardize image tags as immutable `git-<shortsha>`.
- Update deployment docs for dual-pipeline operations and Harbor-only production rollout.
- Document manual dual-remote push flow (`push_both_manual`).

### Out of scope

- Auto mirroring between GitHub and GitLab repositories.
- Replacing GitHub Actions with GitLab-only pipeline.
- Production source builds.

## Architecture

### CI topology

- **GitHub Actions (existing):**
  - Trigger: push/PR/tag on GitHub.
  - Outputs: `ghcr.io/wendal/sip3/backend:<tag>`, `ghcr.io/wendal/sip3/frontend:<tag>`.

- **GitLab CI (new):**
  - Trigger: push/tag on GitLab (`ssh://git@sh.air32.cn:222/wendal/sip3.git`).
  - Runner: self-hosted GitLab runner in China.
  - Outputs: `harbor.air32.cn/sip3/backend:<tag>`, `harbor.air32.cn/sip3/frontend:<tag>`.

- **Production (`sip.air32.cn:/opt/sip3`):**
  - Deploy source: Harbor only.
  - Rollout: update `IMAGE_TAG` in `.env`, then `docker compose pull && docker compose up -d`.

### Tagging policy

- Primary immutable release tag: `git-<shortsha>`.
- Optional semantic tag (`v*`) can be published in both registries for releases.
- `latest` is not used for production deployment.

## GitLab CI Design

## Pipeline stages

1. `backend_test`
   - `cd backend`
   - `cargo fmt --check`
   - `cargo build --verbose`
   - `cargo test --verbose`
   - `cargo clippy -- -D warnings`

2. `frontend_build`
   - `cd frontend`
   - `npm ci`
   - `npm run build`

3. `docker_publish` (depends on prior stages)
   - Login Harbor with CI variables.
   - Build backend image from `docker/Dockerfile.backend`.
   - Build frontend image from `docker/Dockerfile.frontend`.
   - Push both images to Harbor with `git-$CI_COMMIT_SHORT_SHA`.

## Required GitLab CI Variables

- `HARBOR_REGISTRY=harbor.air32.cn`
- `HARBOR_NAMESPACE=sip3`
- `HARBOR_USER`
- `HARBOR_PASSWORD`

Optional:

- `IMAGE_TAG_OVERRIDE` (for manual release/tag override workflows).

## Deployment Flow (Production)

1. Ensure `.env` has:
   - `HARBOR_IMAGE_PREFIX=harbor.air32.cn/sip3`
   - `IMAGE_TAG=git-<shortsha>`
2. Deploy:
   - `docker compose pull`
   - `docker compose up -d`
   - `docker compose ps`
3. Health checks:
   - `curl -f http://127.0.0.1:3000/api/health`
   - frontend HTTP check via host nginx endpoint.

## Rollback

1. Set `.env` `IMAGE_TAG` back to last known good tag.
2. Execute:
   - `docker compose pull`
   - `docker compose up -d`
3. Re-run health checks.

## Operational Workflow (push_both_manual)

Developers push to both remotes explicitly:

- `git push origin <branch-or-tag>` (GitHub)
- `git push gitlab <branch-or-tag>` (GitLab)

This triggers GitHub and GitLab pipelines independently.

## Failure Handling

- If GitHub CI fails: overseas publishing path is affected; domestic GitLab+Harbor path remains available.
- If GitLab CI or Harbor push fails: domestic deploy for new tag is blocked; production remains on current tag.
- If production pull fails: no deployment proceeds; rollback by keeping previous tag.

## Acceptance Criteria

1. GitHub Actions still passes and can publish GHCR images.
2. GitLab CI passes on self-hosted runner and publishes Harbor images with immutable tags.
3. Production server can deploy and roll back using Harbor tags only.
4. Deployment docs clearly describe dual-pipeline behavior and manual dual-remote push operations.

