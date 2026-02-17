import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import { randomBytes } from 'crypto';

// --- Credentials loaded from environment variables ---
// Set ADMIN_USERNAME and ADMIN_PASSWORD in your .env.local file or deployment environment.
// The server will refuse to start auth functions if ADMIN_PASSWORD is not set.
const ADMIN_USERNAME = process.env.ADMIN_USERNAME ?? 'admin';
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD;

// Cookie name for the session token
const SESSION_COOKIE = 'ragmcp_session';

// --- Token store singleton ---
// IMPORTANT: This must live on `globalThis`, NOT as a plain module-level variable.
//
// In Next.js 15 App Router, every route file is compiled as an isolated module.
// A plain `const VALID_TOKENS = new Set()` would create a *separate* Set per
// route — so a token added in /api/auth/login/route.ts would be invisible to
// /api/documents/route.ts when it calls isAuthenticated().
//
// `globalThis` is shared across all modules within the same Node.js process,
// which is the standard Next.js pattern for in-memory singletons (e.g. the
// Prisma client example in the official docs).
//
// For multi-process / edge deployments, replace with a shared store (Redis, DB).
declare global {
  // eslint-disable-next-line no-var
  var __ragmcpValidTokens: Set<string> | undefined;
}
globalThis.__ragmcpValidTokens ??= new Set<string>();
const VALID_TOKENS = globalThis.__ragmcpValidTokens;

/**
 * Attempts login with the given credentials.
 * Returns true and sets a secure session cookie on success.
 * Returns false if credentials don't match or ADMIN_PASSWORD is not configured.
 *
 * @param username - Username submitted by the user
 * @param password - Password submitted by the user
 */
export async function login(username: string, password: string): Promise<boolean> {
  // Guard: ADMIN_PASSWORD must be configured via environment variable
  if (!ADMIN_PASSWORD) {
    console.error(
      '[auth] ADMIN_PASSWORD environment variable is not set. ' +
      'Set it in .env.local before running the dashboard.'
    );
    return false;
  }

  if (username === ADMIN_USERNAME && password === ADMIN_PASSWORD) {
    // Generate a cryptographically random 32-byte token (256-bit entropy).
    // This is far more secure than a fixed string like "authenticated".
    const token = randomBytes(32).toString('hex');
    VALID_TOKENS.add(token);

    const cookieStore = await cookies();
    // Set secure httpOnly session cookie (expires in 24 hours)
    cookieStore.set(SESSION_COOKIE, token, {
      httpOnly: true,
      secure: process.env.NODE_ENV === 'production',
      sameSite: 'strict',
      maxAge: 60 * 60 * 24, // 24 hours
      path: '/',
    });
    return true;
  }
  return false;
}

/**
 * Clears the session cookie and invalidates the token, logging the user out.
 */
export async function logout() {
  const cookieStore = await cookies();
  const token = cookieStore.get(SESSION_COOKIE)?.value;
  // Invalidate the token server-side so it cannot be reused after logout
  if (token) {
    VALID_TOKENS.delete(token);
  }
  cookieStore.delete(SESSION_COOKIE);
}

/**
 * Returns true if the current request has a valid session token.
 * Validates against the in-memory token store (not just the cookie value).
 */
export async function isAuthenticated(): Promise<boolean> {
  const cookieStore = await cookies();
  const token = cookieStore.get(SESSION_COOKIE)?.value;
  // Token must exist AND be in the valid set (prevents forged/replayed cookies)
  return !!token && VALID_TOKENS.has(token);
}

/**
 * Redirects to /login if the user is not authenticated.
 * Call this at the top of any server component or route that requires auth.
 */
export async function requireAuth() {
  if (!(await isAuthenticated())) {
    redirect('/login');
  }
}
