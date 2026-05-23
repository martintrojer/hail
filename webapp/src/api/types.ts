// This file is auto-generated from src/api/openapi.example.json.
// Do not edit by hand; regenerate via npm run api:types.
export interface paths {
    "/api/compose": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["compose"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/drafts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["create_draft"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/drafts/{draft_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch: operations["update_draft"];
        trace?: never;
    };
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
    "/api/threads/{thread_id}/reply": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["reply"];
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
        ComposePayload: {
            attachments?: unknown[] | null;
            bcc?: string[] | null;
            body_markdown: string;
            cc?: string[] | null;
            /** Format: date-time */
            send_at?: string | null;
            subject: string;
            to: string[];
        };
        ComposeResponse: {
            draft_email_id: string;
            /** Format: int64 */
            scheduled_send_id: number;
            /** @enum {string} */
            status: "pending";
        } | {
            email_id: string;
            /** @enum {string} */
            status: "sent";
            submission_id?: string | null;
        };
        DraftPayload: {
            attachments?: unknown[] | null;
            bcc?: string[] | null;
            body_markdown?: string | null;
            cc?: string[] | null;
            subject?: string | null;
            to?: string[] | null;
        };
        DraftResponse: {
            draft_id: string;
            /** Format: date-time */
            updated_at: string;
        };
        Participant: {
            email: string;
            name?: string | null;
        };
        ReplyPayload: {
            attachments?: unknown[] | null;
            body_markdown: string;
            /** Format: date-time */
            send_at?: string | null;
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
    compose: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ComposePayload"];
            };
        };
        responses: {
            /** @description Message sent immediately. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ComposeResponse"];
                };
            };
            /** @description Message draft scheduled for later delivery. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ComposeResponse"];
                };
            };
            /** @description Invalid compose payload. */
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
            /** @description JMAP provider or scheduler failure. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    create_draft: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DraftPayload"];
            };
        };
        responses: {
            /** @description Draft created or autosaved. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DraftResponse"];
                };
            };
            /** @description Invalid draft payload. */
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
            /** @description JMAP draft store failure. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    update_draft: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP draft email id to update. */
                draft_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DraftPayload"];
            };
        };
        responses: {
            /** @description Draft updated. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DraftResponse"];
                };
            };
            /** @description Invalid draft id or payload. */
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
            /** @description JMAP draft store failure. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
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
    reply: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id to reply to. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ReplyPayload"];
            };
        };
        responses: {
            /** @description Reply sent immediately. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ComposeResponse"];
                };
            };
            /** @description Reply draft scheduled for later delivery. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ComposeResponse"];
                };
            };
            /** @description Invalid thread id or reply payload. */
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
            /** @description JMAP provider or scheduler failure. */
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
