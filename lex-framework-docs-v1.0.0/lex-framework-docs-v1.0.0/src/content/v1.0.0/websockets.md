# WebSockets

`lex_rest` contains a WebSocket/WSS boundary alongside the HTTP REST layer.

Use WebSockets when your application requires server-initiated realtime communication. Authentication, authorization, origin policy, message-size limits, connection lifetime, rate controls, and secure transport should be treated as part of the WebSocket security boundary—not as optional HTTP-only concerns.

For production, use WSS or a trusted TLS-terminating edge and preserve the same identity/tenant rules that protect normal HTTP APIs.
