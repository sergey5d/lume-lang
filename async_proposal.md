# Async / Concurrency Proposal

This file is for unsettled async and concurrency language ideas.

## Structured Forks

The first useful capability is overlapping independent work while keeping the
control flow local and explicit. For example, in Contado's reports controller,
three report queries currently run sequentially, which means the endpoint pays
roughly three database round trips. With `fork`, those queries can overlap:

```lume
@Get { path: "/reports/summary", contentType: "application/json" }
def summary(ctx Context) Result[HttpResponse, HttpError] {
    orgId OrgId = try this.orgId(ctx)

    scope {
        revenueTask = fork this.repo().revenueTotals(orgId)
        expenseTask = fork this.repo().expenseTotals(orgId)
        agingTask   = fork this.repo().arAging(orgId)

        revenue = try revenueTask.join().mapError { error -> ApiResult.httpError(500, error.toStr()) }
        expenses = try expenseTask.join().mapError { error -> ApiResult.httpError(500, error.toStr()) }
        aging = try agingTask.join().mapError { error -> ApiResult.httpError(500, error.toStr()) }

        Ok({ body: Json.stringify({ revenue: revenue, expenses: expenses, aging: aging }) })
    }
}
```

Mental model:

- `fork expr` starts `expr` in a child task.
- Each fork can run on a virtual thread.
- Each database query grabs its own Hikari connection, so independent queries can genuinely run at the same time.
- `scope { ... }` guarantees all child tasks finish or are cancelled before the block exits.
- `join()` returns through the language's normal `Result` flow, so failures can be mapped before `try`.

This keeps parallelism explicit without making the endpoint callback-shaped.

Open questions:

- Exact task type name and generic shape, for example `Task[T, E]` versus `Fiber[T, E]`.
- Whether `fork` accepts only expressions returning `Result` or any expression.
- Whether child failures cancel sibling tasks automatically or only when the scope exits.
- Whether `scope` is expression-valued like ordinary blocks.

## Keepers

If the runtime later needs safe in-process shared state, a `keeper` construct
could model serialized mutable state. For example, Contado auth caching could
avoid re-validating the same token on every request:

```lume
keeper AuthCache {
    entries Map[Str, CachedAuth] = Map()

    def lookup(token Str) Option[CachedAuth] =
        entries[token]

    def store(token Str, auth CachedAuth) Unit {
        entries.put(token, auth)
    }
}
```

Callers would use it like a singleton:

```lume
cached Option[CachedAuth] = AuthCache.lookup(token)
```

Mental model:

- A `keeper` owns mutable state.
- Calls into a keeper are serialized by the runtime.
- The mutable map inside the keeper stays safe even when many HTTP handlers call it concurrently.
- Keepers are for shared process-local coordination, not ordinary domain objects.

Open questions:

- Whether `keeper` is a new declaration kind or sugar over `single` plus synchronization.
- Whether keeper methods may be async/forking themselves.
- Whether keeper state is always process-local or can later be backed by a distributed implementation.
