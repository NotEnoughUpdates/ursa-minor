# Ursa Minor

> <i>The small bear - A rather primitive API proxy for the Hypixel public API.</i>

## Server Usage

### Configuration

The server is configured via environment variables. See `.env.example` for explanations what each variable does.
Environment variables in `.env` get automatically loaded on startup. Rules are resolved relative to the working
directory.

Also check out [ursa-minor-stat-viewer](https://github.com/romangraef/ursa-minor-stat-viewer-stats) for stat aggregation.

## Client Usage

## Requesting a resource

You will need to send a GET request to `/v1/hypixel/<rulename>/<ruleArg1>/<ruleArg2>`

### Authentication

Clients may need to provide authentication in form of an associated minecraft account for some routes (unless
`URSA_ALLOW_ANONYMOUS` is set). Said authentication may come in two forms:

The client first sends a joinServer request to Mojangs session server: 

```java
var serverId = UUID.randomUUID().toString();
var session = Minecraft.getMinecraft().getSession();
var name = session.getUsername();
request.header("x-ursa-username", name).header("x-ursa-serverid", serverId);
Minecraft.getMinecraft().sessionService.joinServer(session.getProfile(), session.getToken(), serverId);
```

Then the headers `x-ursa-username` and `x-ursa-serverid` need to be set on the next request to the ursa server.
The server will then set the `x-ursa-token` header in the response, which can be used for 1 hour, which can be used
instead of the joinServer authentication:

```java
// ... Do a joinServer request
var tokenFromLastRequest = response.getHeader("x-ursa-token");
// In the next request
request.header("x-ursa-token", tokenFromLastRequest);
```

## Rule format

```json5
{
  // The path that users access the rule on. Will be prefixed with /v1/hypixel/.
  "http-path": "player",
  // The path that is being proxied
  "hypixel-path": "https://api.hypixel.net/player",
  // A list of query arguments that the Hypixel API endpoint needs. Users provide these in the same order they occur
  // in here as subpaths. In this case a full request would be http://ursa.notenoughupdates.org/v1/hypixel/<uuid>
  // This is to make caching by just path possible.
  "query-arguments": [
    "uuid"
  ]
}
```
