# API Response Pattern Examples

camelCase JSON. Lists use root `{ items, total }`; details use `{ data }`; deletes return `204`.

## 1. List: `GET /api/v1/widgets`

```json
{
  "items": [
    {
      "id": "0193b3c4-5e6f-7a8b-9c0d-1e2f3a4b5c6d",
      "name": "Sample",
      "createdAt": "2026-01-15T12:00:00Z"
    }
  ],
  "total": 1
}
```

Bruno:

```bru
assert {
  res.status: eq 200
  res.body.items: isArray
  res.body.total: gte 1
}
```

## 2. Details: `GET /api/v1/widgets/{widgetId}`

```json
{
  "data": {
    "id": "0193b3c4-5e6f-7a8b-9c0d-1e2f3a4b5c6d",
    "name": "Sample",
    "createdAt": "2026-01-15T12:00:00Z",
    "updatedAt": "2026-01-16T08:30:00Z"
  }
}
```

Bruno:

```bru
assert {
  res.status: eq 200
  res.body.data.id: isDefined
  res.body.data.name: eq Sample
}
```

## 3. Nullable Details: Valid Absence

```json
{
  "data": null
}
```

HTTP `200` when the parent exists but the optional child does not.

## 4. Delete: `DELETE /api/v1/widgets/{widgetId}`

HTTP `204`, empty body.

```bru
assert {
  res.status: eq 204
}
```

## 5. Health: `GET /health`

```json
{
  "status": "healthy"
}
```

Not wrapped in `{ data }`. Readiness (`GET /health/ready`) adds `"database": "up"` and returns `503` with `"status": "unhealthy", "database": "down"` when Postgres is unreachable.

## 6. Error: Validation

HTTP `400`:

```json
{
  "error": "Name is required.",
  "code": "VALIDATION_ERROR",
  "field": "name",
  "requestId": "6f1c2d3e-4b5a-6978-8a9b-0c1d2e3f4a5b"
}
```

## 7. Error: Not Found

HTTP `404`:

```json
{
  "error": "Widget with id '0193b3c4' not found",
  "code": "ENTITY_NOT_FOUND",
  "details": { "entityName": "Widget", "entityId": "0193b3c4" },
  "requestId": "6f1c2d3e-4b5a-6978-8a9b-0c1d2e3f4a5b"
}
```

## 8. Error: Unauthorized

HTTP `401` (auth enabled, token rejected):

```json
{
  "error": "Unauthorized",
  "code": "UNAUTHORIZED",
  "requestId": "6f1c2d3e-4b5a-6978-8a9b-0c1d2e3f4a5b"
}
```
