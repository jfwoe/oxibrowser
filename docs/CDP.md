# CDP Protocol Reference

OxiBrowser implements the Chrome DevTools Protocol (CDP) over WebSocket,
providing compatibility with Puppeteer, Playwright, and other CDP clients.

## Connection

### HTTP Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /json/version` | Browser version info |
| `GET /json` | Available targets |
| `GET /` | Simple HTML status page |

### WebSocket

Connect to `ws://host:port/ws` to send CDP commands.

## Protocol Format

### Request

```json
{
    "id": 1,
    "method": "Page.navigate",
    "params": {
        "url": "https://example.com"
    }
}
```

### Response

```json
{
    "id": 1,
    "result": {
        "frameId": "frame-1",
        "loaderId": "loader-1"
    }
}
```

### Error

```json
{
    "id": 1,
    "error": {
        "code": -32600,
        "message": "Invalid request"
    }
}
```

### Events

```json
{
    "method": "Page.frameNavigated",
    "params": {
        "frame": {
            "id": "frame-1",
            "url": "https://example.com"
        }
    }
}
```

## Domain Reference

### Browser

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Browser.getVersion` | — | Returns browser version info |
| `Browser.close` | — | Closes the browser |
| `Browser.setWindowBounds` | `windowId?, width?, height?` | Set viewport override (applies from the next navigation's layout) |
| `Browser.getWindowBounds` | `windowId?` | Returns default window bounds |

### DOM

| Method | Parameters | Description |
|--------|-----------|-------------|
| `DOM.getDocument` | `depth?` | Returns root DOM node |
| `DOM.describeNode` | `nodeId` | Returns node info |
| `DOM.querySelector` | `nodeId, selector` | Find first matching element |
| `DOM.querySelectorAll` | `nodeId, selector` | Find all matching elements |
| `DOM.getOuterHTML` | `nodeId` | Get outer HTML of node |
| `DOM.removeAttribute` | `nodeId, name` | Remove an attribute |
| `DOM.setNodeValue` | `nodeId, value` | Set node value |

### Emulation

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Emulation.setDeviceMetricsOverride` | `width, height, ...` | Viewport override (applies from the next navigation) |
| `Emulation.clearDeviceMetricsOverride` | — | Clear viewport override |
| `Emulation.setUserAgentOverride` | `userAgent` | Override UA across transport, `navigator.userAgent`, and stealth profile (empty string clears) |
| `Emulation.setGeolocationOverride` | `latitude, longitude, accuracy?` | Override `navigator.geolocation` |
| `Emulation.clearGeolocationOverride` | — | Clear geolocation override |
| `Emulation.setTimezoneOverride` | `timezoneId` | Override `Intl`/`Date` timezone |
| `Emulation.clearTimezoneOverride` | — | Clear timezone override |
| `Emulation.setEmulatedMedia` | `features` | Only `prefers-color-scheme: dark\|light` is honored (matchMedia immediate; layout from the next document build). Other features are ignored and reported in the `ignored` response field |

### Fetch

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Fetch.enable` | `patterns?, handleAuthRequests?` | Enable request interception |
| `Fetch.disable` | — | Disable interception |
| `Fetch.continueRequest` | `requestId, url?, headers?, postData?` | Continue with modifications |
| `Fetch.failRequest` | `requestId, reason` | Fail the request |
| `Fetch.fulfillRequest` | `requestId, responseCode, responseHeaders, body?` | Return synthetic response |
| `Fetch.getResponseBody` | `requestId` | Get response body |

**Interception flow:**

1. `Fetch.enable({ patterns: [...] })` — start intercepting
2. `Fetch.requestPaused` event fires for each matching request
3. Respond with one of:
   - `Fetch.continueRequest` — allow with optional modifications
   - `Fetch.failRequest` — block the request
   - `Fetch.fulfillRequest` — return a synthetic response

### Input

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Input.dispatchKeyEvent` | `type, key, code, text?` | Dispatch keyboard event |
| `Input.dispatchMouseEvent` | `type, x, y, button?` | Dispatch mouse event |
| `Input.insertText` | `text` | Insert text at cursor |

Key event types: `keyDown`, `keyUp`, `rawKeyDown`, `char`

Mouse event types: `mousePressed`, `mouseReleased`, `mouseMoved`

### Network

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Network.enable` | `maxTotalBufferSize?, maxResourceBufferSize?` | Enable network events |
| `Network.disable` | — | Disable network events |
| `Network.setExtraHTTPHeaders` | `headers` | Set default headers |
| `Network.getResponseBody` | `requestId` | Get response body for request |
| `Network.getAllCookies` | — | Get all cookies |
| `Network.getCookies` | `urls?` | Get cookies for URLs |
| `Network.setCookie` | `name, value, domain?, url?, ...` | Set a cookie |
| `Network.deleteCookies` | `name, domain?, url?` | Delete cookies |
| `Network.setCacheDisabled` | `cacheDisabled` | Accepted no-op — no HTTP cache layer exists |
| `Network.emulateNetworkConditions` | `offline, latency?, ...` | `offline` honored; latency/throughput ignored |
| `Network.getRequestPostData` | `requestId` | POST body of a logged request |

**Events:**

| Event | Description |
|-------|-------------|
| `Network.requestWillBeSent` | HTTP request about to be sent |
| `Network.responseReceived` | HTTP response received |
| `Network.loadingFinished` | Response body fully loaded |
| `Network.loadingFailed` | Request failed (`errorText`) |

### OXI (AI Extensions)

OxiBrowser's proprietary domain for AI agent workflows.

| Method | Parameters | Description |
|--------|-----------|-------------|
| `OXI.getMarkdown` | — | Get page content as markdown |
| `OXI.getPageInfo` | — | Get structured page metadata |
| `OXI.getStructuredPage` | `maxLinks?` | Headings, links, meta as structured JSON |
| `OXI.getAccessibilityTree` | — | Semantic tree (roles, labels, visibility) |
| `OXI.getInteractiveElements` | — | Interactive elements in document order, each with a stable `ref` (`e1`, `e2`, …) for the ref-based methods below; refs go stale when the document changes (generation + fingerprint check) |
| `OXI.clickRef` | `ref` | Click element by ref (stale ref → error `-32000` "re-observe") |
| `OXI.fillRef` | `ref, value` | Fill input/textarea/contentEditable by ref |
| `OXI.waitRef` | `ref, timeoutMs?` | Wait until the ref's element exists |
| `OXI.ariaSnapshot` | — | Playwright-style ARIA snapshot YAML with `[ref=eN]` annotations |
| `OXI.getBoxModelScreenshot` | — | PNG with colored boxes per element (heuristic layout) |
| `OXI.exportStorageState` | — | Playwright-compatible storage state (cookies + localStorage) |
| `OXI.importStorageState` | `state` | Import a storage state snapshot |
| `OXI.getApiGaps` | — | Web APIs accessed by the page but absent here (requires `telemetry: true`) |

**OXI.getMarkdown response:**

```json
{
    "markdown": "# Page Title\n\nPage content in markdown..."
}
```

**OXI.getPageInfo response:**

```json
{
    "url": "https://example.com",
    "title": "Example Domain",
    "statusCode": 200,
    "contentType": "text/html",
    "contentLength": 1256
}
```

### Page

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Page.navigate` | `url, referrer?` | Navigate to URL |
| `Page.reload` | — | Reload the current page |
| `Page.getFrameTree` | — | Get frame tree (includes child iframes) |
| `Page.getTitle` | — | Get page title |
| `Page.captureScreenshot` | `format?, quality?` | Capture screenshot |
| `Page.printToPDF` | `landscape?, margins?` | Print page to PDF (raster-based) |
| `Page.addScriptToEvaluateOnNewDocument` | `source` | Run a script before any page script on every document (returns `identifier`) |
| `Page.removeScriptToEvaluateOnNewDocument` | `identifier` | Remove a previously added init script |
| `Page.getNavigationHistory` | — | Navigation history with `currentIndex` |
| `Page.setLifecycleEventsEnabled` | `enabled` | Enable lifecycle events (buffered for `getLifecycleEvents`) |
| `Page.getLifecycleEvents` | — | Buffered lifecycle events (last 20) |
| `Page.setDownloadBehavior` | `behavior, downloadPath?` | Configure download target directory |
| `Page.handleJavaScriptDialog` | `accept, promptText?` | Resolve a pending `alert`/`confirm`/`prompt` |
| `Page.startScreencast` / `stopScreencast` / `screencastFrameAck` | — | Flow-controlled frame streaming of the live document |

**captureScreenshot** returns:

```json
{
    "data": "iVBORw0KGgoAAAANSUhEUgAA...",
    "metadata": {
        "pageWidth": 800,
        "pageHeight": 600
    }
}
```

`data` is base64-encoded PNG. Only `format: "png"` is supported.

### Runtime

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Runtime.evaluate` | `expression, returnByValue?` | Evaluate JS expression |
| `Runtime.callFunctionOn` | `functionDeclaration, objectId?, arguments?` | Call JS function |
| `Runtime.enable` | — | Enable runtime events |
| `Runtime.disable` | — | Disable runtime events |

**Events:**

| Event | Description |
|-------|-------------|
| `Runtime.executionContextCreated` | New execution context available |
| `Runtime.consoleAPICalled` | `console.log` output |

### Target

| Method | Parameters | Description |
|--------|-----------|-------------|
| `Target.getTargets` | — | List available targets (root + created tabs) |
| `Target.attachToTarget` | `targetId` | Attach to target |
| `Target.detachFromTarget` | `sessionId` | Detach from target (session stays alive) |
| `Target.createTarget` | `url` | Create new target (page) |
| `Target.closeTarget` | `targetId` | Close the target's session (emits `detachedFromTarget` + `targetDestroyed`) |
| `Target.getTargetInfo` | `targetId?` | Target metadata |

## Events

All events are broadcast to all connected WebSocket clients:

| Event | Domain | Description |
|-------|--------|-------------|
| `Page.frameNavigated` | Page | Navigation completed |
| `Page.domContentLoadedEventFired` | Page | DOM content loaded |
| `Page.loadEventFired` | Page | Page fully loaded |
| `Network.requestWillBeSent` | Network | HTTP request initiated |
| `Network.responseReceived` | Network | HTTP response received |
| `Network.loadingFinished` | Network | Response body loaded |
| `Runtime.executionContextCreated` | Runtime | JS context ready |
| `Runtime.consoleAPICalled` | Runtime | Console output |
| `Fetch.requestPaused` | Fetch | Request paused for interception |
| `Target.targetCreated` / `targetDestroyed` | Target | Tab lifecycle |
| `Target.attachedToTarget` | Target | Session attached (carries `sessionId`) |
| `Page.downloadWillBegin` / `Page.downloadProgress` | Page | Download started / completed or failed |
| `Browser.downloadWillBegin` / `Browser.downloadProgress` | Browser | Same download events on the Browser domain (Playwright-compatible) |
| `Page.screencastFrame` | Page | Screencast frame (base64 PNG) |
| `Log.entryAdded` | Log | Console errors mirrored from the JS runtime |
