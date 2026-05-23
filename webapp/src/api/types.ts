// This file is auto-generated from src/api/openapi.example.json.
// Do not edit by hand; regenerate via npm run api:types.
export interface paths {
    "/api/threads/{thread_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_thread"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
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
    schemas: {
        BlockedTrackerResponse: {
            reason: string;
            src: string;
        };
        Participant: {
            email: string;
            name?: string | null;
        };
        ThreadMessageResponse: {
            blocked_trackers: components["schemas"]["BlockedTrackerResponse"][];
            email_id: string;
            from: components["schemas"]["Participant"][];
            html: string;
            preview: string;
            /** Format: date-time */
            received_at?: string | null;
            to: components["schemas"]["Participant"][];
        };
        ThreadViewResponse: {
            messages: components["schemas"]["ThreadMessageResponse"][];
            participants: components["schemas"]["Participant"][];
            subject: string;
            thread_id: string;
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    get_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id to render as a sanitized document. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread rendered as sanitized HTML messages. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadViewResponse"];
                };
            };
            /** @description Invalid thread id. */
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Thread not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Thread assembly failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
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
