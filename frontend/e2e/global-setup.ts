import { waitForReadiness } from "./support/readiness";

/**
 * Runs once before any test. Fails fast with actionable output when the local
 * frontend/backend stack is not reachable, instead of letting every spec time
 * out against a missing server.
 */
export default async function globalSetup(): Promise<void> {
  await waitForReadiness();
}
