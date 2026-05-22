// This file is auto-generated from src/api/openapi.example.json.
// Do not edit by hand; regenerate via npm run api:types.
export interface paths {
    "/healthz": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Liveness probe.
         * @description Returns 204 No Content if the process is up enough to accept HTTP.
         *     Intended for container orchestrators (Kubernetes `livenessProbe`,
         *     Docker `HEALTHCHECK`, etc.) — a failing liveness check should
         *     restart the container.
         */
        get: operations["healthz"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/readyz": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Readiness probe.
         * @description Returns 200 OK once the SQLite pool answers `SELECT 1`, otherwise
         *     503 Service Unavailable. Designed for `readinessProbe` style
         *     gating: a failing readiness check should remove the instance from
         *     load-balancer rotation but *not* restart it.
         *
         *     Per design.md §7.5 the production check will also verify the JMAP
         *     session; that's wired in by the `jmap-eventsource` task.
         */
        get: operations["readyz"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: never;
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    healthz: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Process is alive. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    readyz: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description All dependencies reachable. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description A dependency is unhealthy. */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
}
