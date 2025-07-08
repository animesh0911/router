# CORS with Trusted Origins

This example demonstrates how to use the new `trusted_origins` feature in the Apollo Router CORS plugin.

## Problem

You have a publicly accessible GraphQL gateway that needs to:
1. Allow CORS requests from any origin (for public access)
2. Only allow credentials (cookies, authentication headers) from specific trusted domains
3. Maintain security by preventing credential leakage to untrusted domains

## Solution

The `trusted_origins` configuration option allows you to specify a list of domains that are allowed to send credentials, while still enabling CORS for all other domains.

## Configuration

```yaml
# router.yaml
cors:
  trusted_origins:
    - "https://app.mycompany.com"
    - "https://admin.mycompany.com"
    - "https://dashboard.mycompany.com"
  allow_headers:
    - "content-type"
    - "authorization"
    - "x-apollo-operation-name"
  expose_headers:
    - "x-custom-header"
  methods:
    - "GET"
    - "POST"
    - "OPTIONS"
  max_age: "86400"
```

## How it works

### For Trusted Origins

When a request comes from a trusted origin (e.g., `https://app.mycompany.com`):

**Request:**
```http
GET /graphql
Origin: https://app.mycompany.com
Cookie: session=abc123
```

**Response:**
```http
Access-Control-Allow-Origin: https://app.mycompany.com
Access-Control-Allow-Credentials: true
Access-Control-Expose-Headers: x-custom-header
```

### For Untrusted Origins

When a request comes from an untrusted origin (e.g., `https://example.com`):

**Request:**
```http
GET /graphql
Origin: https://example.com
Cookie: session=abc123
```

**Response:**
```http
Access-Control-Allow-Origin: *
Access-Control-Expose-Headers: x-custom-header
```

Note: No `Access-Control-Allow-Credentials` header is sent, so the browser will not send cookies or authentication headers.

### For Preflight Requests

#### Trusted Origin Preflight
```http
OPTIONS /graphql
Origin: https://app.mycompany.com
Access-Control-Request-Method: POST
Access-Control-Request-Headers: content-type
```

Response:
```http
Access-Control-Allow-Origin: https://app.mycompany.com
Access-Control-Allow-Credentials: true
Access-Control-Allow-Methods: GET, POST, OPTIONS
Access-Control-Allow-Headers: content-type
Access-Control-Max-Age: 86400
```

#### Untrusted Origin Preflight
```http
OPTIONS /graphql
Origin: https://untrusted.com
Access-Control-Request-Method: POST
Access-Control-Request-Headers: content-type
```

Response:
```http
Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET, POST, OPTIONS
Access-Control-Allow-Headers: content-type
Access-Control-Max-Age: 86400
```

## Key Benefits

1. **Security**: Credentials are only sent to trusted domains
2. **Flexibility**: Any domain can make CORS requests (good for public APIs)
3. **Compliance**: Meets CORS specification requirements
4. **Performance**: Preflight requests are handled efficiently

## Migration from Standard CORS

If you're currently using:

```yaml
cors:
  allow_any_origin: true
  allow_credentials: true  # This is invalid!
```

You can migrate to:

```yaml
cors:
  trusted_origins:
    - "https://yourapp.com"
    - "https://youradmin.com"
  # Other settings remain the same
```

## Configuration Validation

The following configurations are invalid when using `trusted_origins`:

```yaml
cors:
  trusted_origins:
    - "https://trusted.com"
  allow_any_origin: true  # ❌ Cannot combine with trusted_origins
```

```yaml
cors:
  trusted_origins:
    - "*"  # ❌ Cannot use wildcard in trusted_origins
```

## Use Cases

### SaaS Application
```yaml
cors:
  trusted_origins:
    - "https://app.mycompany.com"
    - "https://admin.mycompany.com"
  # Allows public documentation sites to make requests without credentials
```

### Multi-tenant Platform
```yaml
cors:
  trusted_origins:
    - "https://tenant1.myplatform.com"
    - "https://tenant2.myplatform.com"
    - "https://admin.myplatform.com"
  # Allows third-party integrations to access public data
```

### Development Environment
```yaml
cors:
  trusted_origins:
    - "http://localhost:3000"
    - "http://localhost:8080"
    - "https://preview.myapp.com"
  # Allows development servers to authenticate while keeping API public
``` 