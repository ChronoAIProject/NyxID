# Mobile App Integration Guide

This guide explains how to integrate NyxID authentication into native mobile apps (iOS, Android, Flutter, React Native) using the Social Token Exchange flow. This avoids browser redirects and provides a native sign-in experience identical to Firebase Auth.

---

## Table of Contents

- [Overview](#overview)
- [How It Works](#how-it-works)
- [Prerequisites](#prerequisites)
- [Google Sign-In](#google-sign-in)
  - [iOS (Swift)](#ios-swift)
  - [Android (Kotlin)](#android-kotlin)
  - [Flutter](#flutter)
  - [React Native](#react-native)
- [GitHub Sign-In](#github-sign-in)
- [Token Management](#token-management)
  - [Storing Tokens](#storing-tokens)
  - [Refreshing Tokens](#refreshing-tokens)
  - [Making Authenticated Requests](#making-authenticated-requests)
  - [Logging Out](#logging-out)
- [API Reference](#api-reference)
  - [Exchange Token](#exchange-token)
  - [Refresh Token](#refresh-token)
  - [Get User Profile](#get-user-profile)
- [Error Handling](#error-handling)
- [Account Linking](#account-linking)
- [Security Considerations](#security-considerations)
- [Migrating from Firebase Auth](#migrating-from-firebase-auth)

---

## Overview

NyxID supports two authentication flows for different client types:

| Flow | Client Type | User Experience |
|------|------------|-----------------|
| OIDC Authorization Code (PKCE) | Web apps, desktop, MCP clients | Redirects to NyxID login page in browser |
| **Social Token Exchange** | **Native mobile apps** | **Native SDK popup, no browser redirect** |

The Social Token Exchange flow uses [RFC 8693 (OAuth 2.0 Token Exchange)](https://datatracker.ietf.org/doc/html/rfc8693) to accept provider tokens obtained through native SDKs and exchange them for NyxID tokens.

```
Mobile App                         NyxID                     Google/GitHub
    |                                |                            |
    |  1. Native SDK sign-in         |                            |
    |  (Google Sign-In popup)        |                            |
    |------------------------------->|                            |
    |  Google ID token               |                            |
    |                                |                            |
    |  2. POST /oauth/token          |                            |
    |  (token exchange request)      |                            |
    |------------------------------->|                            |
    |                                |  3. Verify token           |
    |                                |  (JWKS / API validation)   |
    |                                |--------------------------->|
    |                                |  4. Valid                  |
    |                                |<---------------------------|
    |                                |                            |
    |                                |  5. Find or create user    |
    |                                |  6. Issue NyxID tokens     |
    |                                |                            |
    |  7. NyxID tokens               |                            |
    |  (access + refresh + id_token) |                            |
    |<-------------------------------|                            |
    |                                |                            |
    |  8. Use access_token for       |                            |
    |     all NyxID API calls        |                            |
    |------------------------------->|                            |
```

---

## How It Works

1. The mobile app triggers a native sign-in dialog using the platform SDK (Google Sign-In, GitHub OAuth).
2. The user authenticates with the provider. The SDK returns a provider token (Google ID token or GitHub access token).
3. The app sends this token to NyxID's token endpoint (`POST /oauth/token`) with `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`.
4. NyxID verifies the token:
   - **Google**: Validates the JWT signature against Google's public keys (JWKS), checks `iss`, `aud`, `exp`, and `email_verified`.
   - **GitHub**: Verifies the token against NyxID's configured GitHub OAuth app, then calls GitHub APIs to retrieve the user profile.
5. NyxID finds or creates the user account (same matching logic as web social login).
6. NyxID returns a full token set: `access_token`, `refresh_token`, and `id_token`.

---

## Prerequisites

### 1. Register an OAuth Client in NyxID

You (or your NyxID admin) need to create an OAuth client for your mobile app. This can be done through the NyxID dashboard:

1. Log in to the NyxID dashboard.
2. Navigate to **Developer Apps** in the sidebar.
3. Click **New Application**.
4. Fill in the form:
   - **Application Name**: A descriptive name for your mobile app (e.g., "Soul Garden iOS").
   - **Redirect URIs**: Add your app's deep link callback URL (e.g., `myapp://oauth/callback`). This is required even though the social token exchange flow does not use redirects -- it may be needed if your app also supports the standard OIDC flow for other providers.
   - **Client Type**: Select **Public (PKCE)**. Mobile apps cannot securely store a client secret, so a public client is the correct choice.
5. Click **Create App**.
6. Copy the **Client ID** shown on the app card. You will need this in your mobile app code.

For confidential clients (server-side apps), a client secret is generated and shown once at creation. For public clients (mobile apps), no secret is generated -- you only need the `client_id`.

To view or edit your app later, click **View Details** on the app card. From the detail page you can update the name, redirect URIs, rotate the client secret (confidential clients only), or deactivate the app.

### 2. Configure Social Providers in NyxID

The NyxID server must have the following environment variables set:

- **Google**: `GOOGLE_CLIENT_ID` (the same Google Cloud OAuth client ID used by your mobile app)
- **GitHub**: `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET`

The `GOOGLE_CLIENT_ID` on the server must match the audience of the Google ID tokens issued to your mobile app. If you use different Google client IDs for iOS and Android, the server must be configured with the one that matches (or use a web client ID as the audience -- see Google's documentation on [cross-client identity](https://developers.google.com/identity/sign-in/web/server-side-flow)).

### 3. Set Up Native SDKs

Follow each provider's setup guide for your platform:

- **Google Sign-In**: [iOS](https://developers.google.com/identity/sign-in/ios/start), [Android](https://developers.google.com/identity/sign-in/android/start)
- **GitHub OAuth**: Use [OctoKit](https://github.com/nicklockwood/OctoKit) or [AppAuth](https://appauth.io/)

---

## Google Sign-In

### iOS (Swift)

```swift
import GoogleSignIn

class AuthManager {
    static let nyxidBaseURL = "https://auth.example.com"
    static let nyxidClientID = "your-nyxid-client-id"

    /// Sign in with Google and exchange the token for NyxID credentials.
    func signInWithGoogle(presenting viewController: UIViewController) async throws -> NyxIDTokens {
        // Step 1: Google native sign-in
        let result = try await GIDSignIn.sharedInstance.signIn(withPresenting: viewController)
        guard let idToken = result.user.idToken?.tokenString else {
            throw AuthError.noIDToken
        }

        // Step 2: Exchange Google ID token for NyxID tokens
        return try await exchangeToken(
            provider: "google",
            subjectToken: idToken,
            subjectTokenType: "urn:ietf:params:oauth:token-type:id_token"
        )
    }

    /// Exchange a provider token for NyxID tokens via RFC 8693 Token Exchange.
    private func exchangeToken(
        provider: String,
        subjectToken: String,
        subjectTokenType: String
    ) async throws -> NyxIDTokens {
        var components = URLComponents()
        components.queryItems = [
            URLQueryItem(name: "grant_type", value: "urn:ietf:params:oauth:grant-type:token-exchange"),
            URLQueryItem(name: "subject_token", value: subjectToken),
            URLQueryItem(name: "subject_token_type", value: subjectTokenType),
            URLQueryItem(name: "client_id", value: Self.nyxidClientID),
            URLQueryItem(name: "provider", value: provider),
        ]

        var request = URLRequest(url: URL(string: "\(Self.nyxidBaseURL)/oauth/token")!)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = components.query?.data(using: .utf8)

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse, httpResponse.statusCode == 200 else {
            let error = try JSONDecoder().decode(OAuthError.self, from: data)
            throw AuthError.exchangeFailed(error.errorDescription)
        }

        return try JSONDecoder().decode(NyxIDTokens.self, from: data)
    }
}

struct NyxIDTokens: Codable {
    let accessToken: String
    let tokenType: String
    let expiresIn: Int
    let refreshToken: String
    let idToken: String?
    let scope: String?

    enum CodingKeys: String, CodingKey {
        case accessToken = "access_token"
        case tokenType = "token_type"
        case expiresIn = "expires_in"
        case refreshToken = "refresh_token"
        case idToken = "id_token"
        case scope
    }
}

struct OAuthError: Codable {
    let error: String
    let errorDescription: String?

    enum CodingKeys: String, CodingKey {
        case error
        case errorDescription = "error_description"
    }
}
```

### Android (Kotlin)

```kotlin
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import com.google.android.libraries.identity.googleid.GetGoogleIdOption
import com.google.android.libraries.identity.googleid.GoogleIdTokenCredential
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.FormBody
import okhttp3.OkHttpClient
import okhttp3.Request

class AuthManager(private val context: Context) {
    companion object {
        const val NYXID_BASE_URL = "https://auth.example.com"
        const val NYXID_CLIENT_ID = "your-nyxid-client-id"
        const val GOOGLE_SERVER_CLIENT_ID = "your-google-web-client-id"
    }

    private val httpClient = OkHttpClient()

    suspend fun signInWithGoogle(activity: Activity): NyxIDTokens {
        // Step 1: Google Credential Manager sign-in
        val credentialManager = CredentialManager.create(context)
        val googleIdOption = GetGoogleIdOption.Builder()
            .setServerClientId(GOOGLE_SERVER_CLIENT_ID)
            .setFilterByAuthorizedAccounts(false)
            .build()

        val request = GetCredentialRequest.Builder()
            .addCredentialOption(googleIdOption)
            .build()

        val result = credentialManager.getCredential(activity, request)
        val credential = GoogleIdTokenCredential.createFrom(result.credential.data)
        val idToken = credential.idToken

        // Step 2: Exchange for NyxID tokens
        return exchangeToken(
            provider = "google",
            subjectToken = idToken,
            subjectTokenType = "urn:ietf:params:oauth:token-type:id_token"
        )
    }

    private suspend fun exchangeToken(
        provider: String,
        subjectToken: String,
        subjectTokenType: String
    ): NyxIDTokens = withContext(Dispatchers.IO) {
        val body = FormBody.Builder()
            .add("grant_type", "urn:ietf:params:oauth:grant-type:token-exchange")
            .add("subject_token", subjectToken)
            .add("subject_token_type", subjectTokenType)
            .add("client_id", NYXID_CLIENT_ID)
            .add("provider", provider)
            .build()

        val request = Request.Builder()
            .url("$NYXID_BASE_URL/oauth/token")
            .post(body)
            .build()

        val response = httpClient.newCall(request).execute()
        if (!response.isSuccessful) {
            val errorBody = response.body?.string()
            throw Exception("Token exchange failed: $errorBody")
        }

        // Parse JSON response into NyxIDTokens data class
        val json = JSONObject(response.body!!.string())
        NyxIDTokens(
            accessToken = json.getString("access_token"),
            refreshToken = json.getString("refresh_token"),
            idToken = json.optString("id_token"),
            expiresIn = json.getInt("expires_in")
        )
    }
}

data class NyxIDTokens(
    val accessToken: String,
    val refreshToken: String,
    val idToken: String?,
    val expiresIn: Int
)
```

### Flutter

```dart
import 'package:google_sign_in/google_sign_in.dart';
import 'package:http/http.dart' as http;
import 'dart:convert';

class NyxIDAuth {
  static const nyxidBaseUrl = 'https://auth.example.com';
  static const nyxidClientId = 'your-nyxid-client-id';

  final GoogleSignIn _googleSignIn = GoogleSignIn(scopes: ['email', 'profile']);

  Future<NyxIDTokens> signInWithGoogle() async {
    // Step 1: Google native sign-in
    final account = await _googleSignIn.signIn();
    if (account == null) throw Exception('Google sign-in cancelled');

    final auth = await account.authentication;
    final idToken = auth.idToken;
    if (idToken == null) throw Exception('No ID token from Google');

    // Step 2: Exchange for NyxID tokens
    return await exchangeToken(
      provider: 'google',
      subjectToken: idToken,
      subjectTokenType: 'urn:ietf:params:oauth:token-type:id_token',
    );
  }

  Future<NyxIDTokens> exchangeToken({
    required String provider,
    required String subjectToken,
    required String subjectTokenType,
  }) async {
    final response = await http.post(
      Uri.parse('$nyxidBaseUrl/oauth/token'),
      headers: {'Content-Type': 'application/x-www-form-urlencoded'},
      body: {
        'grant_type': 'urn:ietf:params:oauth:grant-type:token-exchange',
        'subject_token': subjectToken,
        'subject_token_type': subjectTokenType,
        'client_id': nyxidClientId,
        'provider': provider,
      },
    );

    if (response.statusCode != 200) {
      final error = jsonDecode(response.body);
      throw Exception('Token exchange failed: ${error['error_description']}');
    }

    final json = jsonDecode(response.body);
    return NyxIDTokens.fromJson(json);
  }
}

class NyxIDTokens {
  final String accessToken;
  final String refreshToken;
  final String? idToken;
  final int expiresIn;

  NyxIDTokens({
    required this.accessToken,
    required this.refreshToken,
    this.idToken,
    required this.expiresIn,
  });

  factory NyxIDTokens.fromJson(Map<String, dynamic> json) {
    return NyxIDTokens(
      accessToken: json['access_token'],
      refreshToken: json['refresh_token'],
      idToken: json['id_token'],
      expiresIn: json['expires_in'],
    );
  }
}
```

### React Native

```typescript
import { GoogleSignin } from '@react-native-google-signin/google-signin';

const NYXID_BASE_URL = 'https://auth.example.com';
const NYXID_CLIENT_ID = 'your-nyxid-client-id';

interface NyxIDTokens {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token: string;
  id_token?: string;
  scope?: string;
}

export async function signInWithGoogle(): Promise<NyxIDTokens> {
  // Step 1: Google native sign-in
  await GoogleSignin.hasPlayServices();
  const userInfo = await GoogleSignin.signIn();
  const idToken = userInfo.data?.idToken;
  if (!idToken) throw new Error('No ID token from Google');

  // Step 2: Exchange for NyxID tokens
  return exchangeToken({
    provider: 'google',
    subjectToken: idToken,
    subjectTokenType: 'urn:ietf:params:oauth:token-type:id_token',
  });
}

async function exchangeToken(params: {
  provider: string;
  subjectToken: string;
  subjectTokenType: string;
}): Promise<NyxIDTokens> {
  const body = new URLSearchParams({
    grant_type: 'urn:ietf:params:oauth:grant-type:token-exchange',
    subject_token: params.subjectToken,
    subject_token_type: params.subjectTokenType,
    client_id: NYXID_CLIENT_ID,
    provider: params.provider,
  });

  const response = await fetch(`${NYXID_BASE_URL}/oauth/token`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: body.toString(),
  });

  if (!response.ok) {
    const error = await response.json();
    throw new Error(`Token exchange failed: ${error.error_description}`);
  }

  return response.json();
}
```

---

## GitHub Sign-In

GitHub does not provide a native mobile SDK with ID tokens. Instead, use the GitHub OAuth flow via a web view or system browser, then exchange the resulting access token:

```
POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:token-exchange
&subject_token={github_access_token}
&subject_token_type=urn:ietf:params:oauth:token-type:access_token
&client_id={nyxid_client_id}
&provider=github
```

NyxID first verifies the GitHub access token is issued for NyxID's configured GitHub OAuth app, then calls GitHub's API (`GET /user` and `GET /user/emails`) to retrieve and validate the user profile.

For the GitHub OAuth flow on mobile, consider using [AppAuth](https://appauth.io/) or a custom in-app browser tab (ASWebAuthenticationSession on iOS, Custom Tabs on Android).

---

## Token Management

### Storing Tokens

Store tokens securely using platform-specific secure storage:

| Platform | Storage |
|----------|---------|
| iOS | Keychain Services (`kSecClassGenericPassword`) |
| Android | EncryptedSharedPreferences or Android Keystore |
| Flutter | `flutter_secure_storage` package |
| React Native | `react-native-keychain` package |

Never store tokens in plain UserDefaults, SharedPreferences, or AsyncStorage.

### Refreshing Tokens

The `access_token` expires after 15 minutes (900 seconds) by default. Use the `refresh_token` to obtain a new access token:

```
POST /api/v1/auth/refresh
Cookie: nyx_refresh_token={refresh_token}
```

For mobile apps that cannot use cookies, send the refresh token as a Bearer token:

```
POST /api/v1/auth/refresh
Authorization: Bearer {refresh_token}
```

Response:

```json
{
  "access_token": "eyJ...",
  "refresh_token": "eyJ...",
  "expires_in": 900
}
```

Important: NyxID uses **refresh token rotation**. Each refresh returns a new `refresh_token`. Always store the latest refresh token and discard the old one. If a previously used refresh token is detected, all tokens for the session are revoked (replay attack protection).

### Making Authenticated Requests

Include the access token in the `Authorization` header for all NyxID API calls:

```
GET /api/v1/users/me
Authorization: Bearer {access_token}
```

### Logging Out

```
POST /api/v1/auth/logout
Authorization: Bearer {access_token}
```

This revokes the session and invalidates the refresh token. Clear all stored tokens on the client.

---

## API Reference

### Exchange Token

Exchange a provider token for NyxID tokens.

```
POST /oauth/token
Content-Type: application/x-www-form-urlencoded
```

**Parameters:**

| Parameter | Required | Description |
|-----------|----------|-------------|
| `grant_type` | Yes | `urn:ietf:params:oauth:grant-type:token-exchange` |
| `subject_token` | Yes | Provider token (Google ID token JWT or GitHub access token) |
| `subject_token_type` | Yes | `urn:ietf:params:oauth:token-type:id_token` (Google) or `urn:ietf:params:oauth:token-type:access_token` (GitHub) |
| `client_id` | Yes | Your NyxID OAuth client ID |
| `client_secret` | No | Only required for confidential clients (not mobile apps) |
| `provider` | Yes | `google` or `github` |

**Example (curl):**

```bash
curl -X POST https://auth.example.com/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d "grant_type=urn:ietf:params:oauth:grant-type:token-exchange" \
  -d "subject_token=eyJhbGciOiJSUzI1NiIs..." \
  -d "subject_token_type=urn:ietf:params:oauth:token-type:id_token" \
  -d "client_id=my-mobile-app" \
  -d "provider=google"
```

**Success Response (200):**

```json
{
  "access_token": "eyJ...",
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "eyJ...",
  "id_token": "eyJ...",
  "scope": "openid profile email",
  "issued_token_type": "urn:ietf:params:oauth:token-type:access_token"
}
```

**Error Response (400):**

```json
{
  "error": "invalid_grant",
  "error_description": "External token verification failed: token has expired"
}
```

### Refresh Token

See [Refreshing Tokens](#refreshing-tokens) above.

### Get User Profile

```
GET /api/v1/users/me
Authorization: Bearer {access_token}
```

**Response:**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "name": "Jane Doe",
  "avatar_url": "https://lh3.googleusercontent.com/...",
  "email_verified": true,
  "social_provider": "google",
  "created_at": "2026-01-15T10:30:00Z"
}
```

---

## Error Handling

| Error Code | OAuth Error | HTTP Status | Description |
|-----------|-------------|-------------|-------------|
| 6004 | `invalid_grant` | 400 | Provider token is invalid, expired, or has wrong audience |
| 6005 | `invalid_request` | 400 | Provider not supported or not configured on server |
| 6001 | `invalid_grant` | 401 | No verified email found from provider |
| 6002 | `invalid_grant` | 409 | Email already linked to a different social provider |
| 6003 | `invalid_grant` | 403 | User account is deactivated |

Common causes and fixes:

- **`token has expired`**: The Google ID token has a short lifespan (~1 hour). Exchange it immediately after sign-in, not after a delay.
- **`Token is too old`**: NyxID rejects Google tokens with `iat` older than 10 minutes. Ensure the token was freshly obtained.
- **`Audience mismatch`**: The `GOOGLE_CLIENT_ID` on the NyxID server does not match the client ID used by your mobile app. They must be the same, or use a web client ID as the audience.
- **`Unsupported or unconfigured provider`**: The server does not have `GOOGLE_CLIENT_ID` or `GITHUB_CLIENT_ID` configured.
- **`Email not verified by Google`**: The Google account has an unverified email. Only verified emails are accepted.

---

## Account Linking

NyxID uses the same account matching logic for both web social login and mobile token exchange:

1. **Returning user**: If a user with the same provider + provider ID exists, they are logged in.
2. **Email linking**: If a user with the same email exists (registered via email/password or another provider), the social identity is linked to the existing account. This only works if the existing account has no social provider set.
3. **New user**: If no match is found, a new account is created with `email_verified = true`.

If the email is already associated with a *different* social provider (e.g., the user signed up with GitHub and now tries Google with the same email), the exchange returns a `409 Conflict` error.

---

## Security Considerations

1. **Public client**: Mobile apps use a `public` OAuth client (no client secret). This is standard practice -- the security relies on the provider token's short lifespan and the secure transport (HTTPS).

2. **Token storage**: Always use platform secure storage (Keychain, Keystore). Never log tokens or store them in plaintext.

3. **Certificate pinning**: Consider implementing certificate pinning for the NyxID domain in production apps to prevent MITM attacks.

4. **Token freshness**: Exchange the provider token immediately after the user signs in. Google ID tokens have a ~1 hour lifespan but NyxID rejects tokens with `iat` older than 10 minutes.

5. **Refresh token rotation**: NyxID rotates refresh tokens on every use. If a stale refresh token is replayed, all session tokens are revoked. Handle `401` responses gracefully by redirecting to sign-in.

6. **No client secret on device**: Never embed a confidential client secret in a mobile app binary. Use a `public` client type.

---

## Migrating from Firebase Auth

If your mobile app currently uses Firebase Auth with Google Sign-In, the migration is minimal:

### What Changes

| Before (Firebase) | After (NyxID) |
|---|---|
| `Firebase.auth().signIn(with: credential)` | `POST /oauth/token` (token exchange) |
| Firebase ID token in `Authorization` header | NyxID access token in `Authorization` header |
| Firebase token refresh (automatic via SDK) | Manual refresh via `POST /api/v1/auth/refresh` |
| `securetoken.google.com/{project_id}` as JWT issuer | NyxID's `JWT_ISSUER` (default: `nyxid`) as JWT issuer |
| Firebase user UID | NyxID user UUID |

### What Stays the Same

- Google Sign-In SDK setup (same `GoogleService-Info.plist` / `google-services.json`)
- Native sign-in UI (same popup, same user experience)
- The Google ID token obtained from the SDK is the same

### Step-by-Step Migration

1. **Keep** your Google Sign-In SDK configuration unchanged.
2. **Replace** the Firebase `signIn(with: credential)` call with a `POST /oauth/token` call to NyxID (see examples above).
3. **Replace** the Firebase token storage with secure storage of NyxID's `access_token` and `refresh_token`.
4. **Replace** `Authorization: Bearer {firebase_id_token}` with `Authorization: Bearer {nyxid_access_token}` in all API calls.
5. **Add** token refresh logic using `POST /api/v1/auth/refresh` (Firebase SDK did this automatically).
6. **Update** your backend to verify NyxID JWTs instead of Firebase JWTs:
   - Change the expected JWT issuer from `securetoken.google.com/{project_id}` to your NyxID issuer
   - Fetch JWKS from NyxID's `/.well-known/jwks.json` instead of Google's `securetoken` endpoint
