import type { Request, Response, NextFunction } from 'express';
import type { NyxIdClient } from './nyxid-client.js';
import type { ProtectedResourceMetadata, UserInfo } from './types.js';

export interface AuthContext {
  readonly token: string;
  readonly user: UserInfo;
}

export function extractBearerToken(req: Request): string | null {
  const authHeader = req.headers.authorization;
  if (!authHeader?.startsWith('Bearer ')) {
    return null;
  }
  return authHeader.slice(7);
}

export function createProtectedResourceMetadata(
  resourceUrl: string,
  authorizationServerUrl: string,
): ProtectedResourceMetadata {
  return {
    resource: resourceUrl,
    authorization_servers: [authorizationServerUrl],
    scopes_supported: ['openid', 'profile', 'email'],
    bearer_methods_supported: ['header'],
  };
}

export function createBearerAuthMiddleware(
  client: NyxIdClient,
  resourceMetadataUrl: string,
) {
  return async (
    req: Request,
    res: Response,
    next: NextFunction,
  ): Promise<void> => {
    const token = extractBearerToken(req);

    if (!token) {
      res
        .status(401)
        .set(
          'WWW-Authenticate',
          `Bearer resource_metadata="${resourceMetadataUrl}"`,
        )
        .json({
          jsonrpc: '2.0',
          error: {
            code: -32001,
            message: 'Authentication required',
          },
          id: null,
        });
      return;
    }

    try {
      const user = await client.getUserInfo(token);
      res.locals.auth = { token, user } satisfies AuthContext;
      next();
    } catch {
      res
        .status(401)
        .set(
          'WWW-Authenticate',
          `Bearer error="invalid_token", resource_metadata="${resourceMetadataUrl}"`,
        )
        .json({
          jsonrpc: '2.0',
          error: {
            code: -32001,
            message: 'Invalid or expired token',
          },
          id: null,
        });
    }
  };
}

export function getAuthContext(res: Response): AuthContext {
  const auth = res.locals.auth as AuthContext | undefined;
  if (!auth) {
    throw new Error('Auth context not available');
  }
  return auth;
}
