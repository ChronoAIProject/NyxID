# Release configured OAuth return pages

Use this procedure for the backend and hosted frontend serving `nyx-api.chrono-ai.fun` and `nyx.chrono-ai.fun`.

1. Merge the reviewed feature through the repository's current `main` workflow after CI passes. Wait for **Publish Images** to finish for that commit. Select both backend and frontend images from that same commit.
2. Set backend `OAUTH_RETURN_ROUTES` to the JSON content in [oauth-return-routes.json](examples/oauth-return-routes.json). Keep `BASE_URL=https://nyx-api.chrono-ai.fun` and `FRONTEND_URL=https://nyx.chrono-ai.fun`. Apply the same return configuration to all replicas.
3. Roll out the backend image using the live deployment's normal process. Confirm that `/health` reports the released commit and `/api/v1/public/config` advertises `oauth_return_routes_enabled: true`. Invalid return configuration prevents startup.
4. Roll out the matching frontend image. This includes dashboard and onboarding connection controls, saved-attempt recovery, and login-continuation fixes. Reload open tabs after rollout so they fetch current code and public configuration.
5. Open [the local POC](http://127.0.0.1:3003/temp). Sign in, choose **Local POC**, and select **Prepare connection**. Verify the saved destination is `http://127.0.0.1:3003/temp`. Open Google setup, complete hosted NyxID login if requested, and consent at Google.
6. Confirm that the browser returns to localhost with the original request ID and the page displays **Google connection confirmed by NyxID**. Inspect the created connection through **View connected service**. A URL status parameter alone does not complete the step.
7. Repeat from the hosted dashboard and onboarding page. Deny Google consent once, retry, and separately cancel a request. Check that a session which expires during setup resumes the original destination after login.
8. For Workspace data access, use the existing scope picker and a bounded Drive read described in [the POC guide](LOCAL_GOOGLE_WORKSPACE_POC.md#nyxid-settings-needed). Current `main` already contains the managed Workspace scope allowlist. Verify the live Google project's API, consent-screen and Workspace admin settings before testing.

Google's connector client must register exactly `https://nyx-api.chrono-ai.fun/api/v1/providers/callback`. Page selection uses NyxID's return map. Registered external apps must also register their application completion URI with NyxID.

For production Docker Compose, the backend already loads `.env.production` through `env_file`. Put `OAUTH_RETURN_ROUTES` there as one quoted JSON value. For another runtime, set the same environment variable through that runtime's existing deployment configuration.

To disable new named requests, remove `OAUTH_RETURN_ROUTES` and restart backend replicas. Existing requests retain their saved destinations until completed, cancelled or expired. Cancel an existing request before removing trust in its destination. Disabling configuration does not cancel issued links.

The live deployment target is not encoded in this repository's image-publishing workflow. Use the actual hosting project's configuration and release mechanism. The legacy Kubernetes section of `DEPLOYMENT.md` references manifests absent from this checkout.
