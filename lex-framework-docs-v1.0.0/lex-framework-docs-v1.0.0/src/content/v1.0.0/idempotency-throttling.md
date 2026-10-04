# Idempotency, throttling, and auditing

The REST layer includes infrastructure contracts for idempotency, distributed throttling, and audit behavior.

## Idempotency

Use idempotency for mutation endpoints where clients may retry the same logical request. The framework provides an idempotency storage abstraction rather than pretending a process-local map gives production guarantees.

For financially important operations, couple HTTP idempotency with authoritative database transaction semantics and an outbox/event design when separate systems are involved.

## Throttling

The framework supports distributed throttling contracts so rate limits can be enforced across multiple application instances when backed by an appropriate shared store.

## Auditing

Audit facilities should capture security-relevant state transitions and mutation context without writing secrets or sensitive payloads into logs.
