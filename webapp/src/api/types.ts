// This file is auto-generated from src/api/openapi.example.json.
// Do not edit by hand; regenerate via npm run api:types.
export interface paths {
    "/api/admin/stats": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_admin_stats"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/blobs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["upload_blobs"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
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
    "/api/contacts/{address}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_contact"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/contacts/{address}/note": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["put_note"];
        post?: never;
        delete: operations["delete_note"];
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
        get: operations["get_draft"];
        put?: never;
        post?: never;
        delete: operations["delete_draft"];
        options?: never;
        head?: never;
        patch: operations["update_draft"];
        trace?: never;
    };
    "/api/screener/decisions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["post_decision"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/screener/{address}/undo-deny": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["post_undo_deny"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
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
    "/api/threads/{thread_id}/archive": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["archive_thread"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/bubble-up": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["bubble_up"];
        delete: operations["cancel_bubble_up"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/classify": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["classify_thread"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/destroy": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete: operations["destroy_thread"];
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/mark": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["mark_thread"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/notes": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_thread_notes"];
        put?: never;
        post: operations["create_thread_note"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/notes/{note_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete: operations["delete_thread_note"];
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
    "/api/threads/{thread_id}/reply-later": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["reply_later"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/restore": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["restore_thread"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/set-aside": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["set_aside"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/trash": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["trash_thread"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/undo/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["post_undo"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/bubble-up": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_bubble_up"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/drafts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_drafts"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/feed": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_feed"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/imbox": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_imbox"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/papertrail": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_papertrail"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/reply-later": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_reply_later"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/screener": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_screener"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/screener/denied": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_denied_senders"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/search": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_search"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/set-aside": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_set_aside"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/views/trash": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_trash"];
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
        AdminStatsResponse: {
            stalwart_status: components["schemas"]["StalwartStatus"];
            users: components["schemas"]["AdminUserStats"][];
        };
        AdminUserStats: {
            email: string;
            /** Format: int64 */
            mailbox_count: number;
            /** Format: int64 */
            total_emails: number;
            /** Format: int64 */
            total_size_bytes?: number | null;
        };
        BlobUploadResponse: {
            blobs: components["schemas"]["UploadedBlob"][];
        };
        BlockedTrackerResponse: {
            reason: string;
            src: string;
        };
        BubbleUpRequest: {
            /** Format: date-time */
            at: string;
        };
        BubbleUpResponse: {
            /** Format: int64 */
            bubble_id: number;
            /** Format: date-time */
            surface_at: string;
        };
        BubbleUpViewItem: {
            /** Format: int64 */
            bubble_id: number;
            /** Format: date-time */
            created_at: string;
            /** Format: date-time */
            surface_at: string;
            thread_id: string;
        };
        BubbleUpViewResponse: {
            items: components["schemas"]["BubbleUpViewItem"][];
        };
        CancelBubbleUpResponse: {
            status: string;
        };
        /** @enum {string} */
        Classification: "imbox" | "feed" | "papertrail";
        ClassifyRequest: {
            to: components["schemas"]["Classification"];
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
        ContactNote: {
            markdown: string;
            /** Format: date-time */
            updated_at: string;
        };
        ContactResponse: {
            address: string;
            note?: null | components["schemas"]["ContactNote"];
            /**
             * @description Placeholder for future contact thread history. The view tasks will
             *     populate this with pre-shaped thread summaries; for now clients get
             *     a stable empty array rather than needing a nullable/missing field.
             */
            threads: unknown[];
        };
        CreateThreadNoteRequest: {
            body: string;
            email_id: string;
        };
        DecisionRequest: {
            apply_to_history: boolean;
            classify_as?: string | null;
            decision: string;
            sender: string;
        };
        DecisionResponse: {
            classify_as?: null | components["schemas"]["Classification"];
            decision: string;
            sender: string;
            undo?: null | components["schemas"]["UndoToken"];
        };
        DeniedSender: {
            /** Format: date-time */
            denied_at: string;
            sender_address: string;
        };
        DeniedSendersResponse: {
            denied: components["schemas"]["DeniedSender"][];
        };
        DestroyThreadResponse: {
            status: string;
        };
        DraftDetails: {
            bcc: string[];
            body_markdown: string;
            cc: string[];
            draft_id: string;
            subject: string;
            to: string[];
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
        /** @enum {string} */
        MailClassification: "imbox" | "feed" | "papertrail" | "drafts" | "trash";
        MailViewItem: {
            bcc: string[];
            cc: string[];
            classification: components["schemas"]["MailClassification"];
            email_id: string;
            from: string;
            has_notes: boolean;
            preview: string;
            /** Format: date-time */
            received_at?: string | null;
            subject: string;
            thread_id: string;
            to: string[];
            unread: boolean;
        };
        MailViewResponse: {
            items: components["schemas"]["MailViewItem"][];
            next_cursor?: string | null;
        };
        MarkRequest: {
            read: boolean;
        };
        Participant: {
            email: string;
            name?: string | null;
        };
        PileItem: {
            /** Format: date-time */
            added_at: string;
            /** Format: int64 */
            position: number;
            preview?: unknown;
            thread_id: string;
        };
        PileViewResponse: {
            items: components["schemas"]["PileItem"][];
        };
        PutNoteRequest: {
            markdown: string;
        };
        ReplyPayload: {
            attachments?: unknown[] | null;
            body_markdown: string;
            /** Format: date-time */
            send_at?: string | null;
        };
        ScreenerEmail: {
            email_id: string;
            preview: string;
            /** Format: date-time */
            received_at?: string | null;
            subject: string;
        };
        ScreenerLatestPreview: {
            from: string;
            preview: string;
            /** Format: date-time */
            received_at?: string | null;
            subject: string;
        };
        ScreenerSender: {
            emails: components["schemas"]["ScreenerEmail"][];
            /** Format: date-time */
            first_seen_at: string;
            latest_preview?: null | components["schemas"]["ScreenerLatestPreview"];
            /** Format: int64 */
            message_count: number;
            sender: string;
        };
        ScreenerViewResponse: {
            senders: components["schemas"]["ScreenerSender"][];
        };
        SearchResponse: {
            results: components["schemas"]["SearchResult"][];
        };
        SearchResult: {
            email_id: string;
            from: string;
            preview: string;
            /** Format: date-time */
            received_at?: string | null;
            subject: string;
            thread_id: string;
            /** @enum {string} */
            type: "mail";
        } | {
            address: string;
            markdown: string;
            /** @enum {string} */
            type: "contact_note";
            /** Format: date-time */
            updated_at: string;
        };
        /** @enum {string} */
        StalwartStatus: "connected" | "unreachable";
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
        ThreadNoteResponse: {
            body: string;
            created_at: string;
            email_id: string;
            /** Format: int64 */
            id: number;
        };
        ThreadNotesResponse: {
            notes: components["schemas"]["ThreadNoteResponse"][];
        };
        ThreadVerbResponse: {
            undo?: null | components["schemas"]["UndoToken"];
        };
        ThreadViewResponse: {
            messages: components["schemas"]["ThreadMessageResponse"][];
            notes: components["schemas"]["ThreadNoteResponse"][];
            participants: components["schemas"]["Participant"][];
            subject: string;
            thread_id: string;
        };
        UndoDenyResponse: {
            status: string;
        };
        UndoResponse: {
            action: string;
            id: string;
        };
        UndoToken: {
            action: string;
            /** Format: date-time */
            expires_at: string;
            id: string;
        };
        UploadedBlob: {
            blob_id: string;
            size: number;
            type: string;
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
    get_admin_stats: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Admin mailbox statistics and Stalwart health status. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AdminStatsResponse"];
                };
            };
            /** @description Authenticated user is not an administrator. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Failed to load local user or session state. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    upload_blobs: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "multipart/form-data": string;
            };
        };
        responses: {
            /** @description Blobs uploaded to JMAP. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BlobUploadResponse"];
                };
            };
            /** @description Invalid multipart upload. */
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
            /** @description Upload too large. */
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Blob upload failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
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
    get_contact: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Email address to inspect. */
                address: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Contact detail with optional note. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ContactResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Contact lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    put_note: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Email address whose note should be saved. */
                address: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["PutNoteRequest"];
            };
        };
        responses: {
            /** @description Contact note saved. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ContactNote"];
                };
            };
            /** @description Invalid contact note payload. */
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
            /** @description Contact note save failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    delete_note: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Email address whose note should be deleted. */
                address: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Contact note deleted. */
            204: {
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
            /** @description Contact note delete failed. */
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
    get_draft: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP draft email id to fetch. */
                draft_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Draft details for composer resume. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DraftDetails"];
                };
            };
            /** @description Invalid draft id. */
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
            /** @description Draft not found or no longer a draft. */
            404: {
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
    delete_draft: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP draft email id to delete. */
                draft_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Draft deleted. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Invalid draft id. */
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
    post_decision: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DecisionRequest"];
            };
        };
        responses: {
            /** @description Screener decision saved. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DecisionResponse"];
                };
            };
            /** @description Invalid screener decision payload. */
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
            /** @description Screener decision failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    post_undo_deny: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Normalized sender address to return to pending screener. */
                address: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Denied sender restored to pending screener. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["UndoDenyResponse"];
                };
            };
            /** @description Invalid sender address. */
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
            /** @description Undo deny failed. */
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
    archive_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread archived. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadVerbResponse"];
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
            /** @description Thread archive failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    bubble_up: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["BubbleUpRequest"];
            };
        };
        responses: {
            /** @description Thread bubble-up scheduled. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BubbleUpResponse"];
                };
            };
            /** @description Invalid bubble-up payload. */
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
            /** @description Bubble-up scheduling failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    cancel_bubble_up: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread bubble-up cancelled. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CancelBubbleUpResponse"];
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
            /** @description Bubble-up cancellation failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    classify_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ClassifyRequest"];
            };
        };
        responses: {
            /** @description Thread reclassified. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadVerbResponse"];
                };
            };
            /** @description Invalid thread id or classification. */
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
            /** @description Thread classify failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    destroy_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread permanently destroyed. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DestroyThreadResponse"];
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
            /** @description Thread destroy failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    mark_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["MarkRequest"];
            };
        };
        responses: {
            /** @description Thread read/unread state updated. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Invalid mark payload. */
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
            /** @description Thread mark failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    list_thread_notes: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id whose notes should be listed. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread notes for this user and thread. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadNotesResponse"];
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
            /** @description Thread note lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    create_thread_note: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id receiving a note. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateThreadNoteRequest"];
            };
        };
        responses: {
            /** @description Thread note created. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadNoteResponse"];
                };
            };
            /** @description Invalid thread note payload. */
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
            /** @description Thread note creation failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    delete_thread_note: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id containing the note. */
                thread_id: string;
                /** @description Thread note id to delete. */
                note_id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread note deleted. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Invalid thread or note id. */
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
            /** @description Thread note deletion failed. */
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
    reply_later: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread added to Reply Later. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadVerbResponse"];
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
            /** @description Reply Later failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    restore_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread restored to inbox. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadVerbResponse"];
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
            /** @description Thread restore failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    set_aside: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread added to Set Aside. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadVerbResponse"];
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
            /** @description Set Aside failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    trash_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Thread moved to trash. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ThreadVerbResponse"];
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
            /** @description Thread trash failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    post_undo: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Opaque 64-character undo token id. */
                id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Undo token consumed and action executed. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["UndoResponse"];
                };
            };
            /** @description Undo payload is invalid. */
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
            /** @description Undo token not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Undo token expired or was already used. */
            410: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Undo execution failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Undo action is not implemented. */
            501: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_bubble_up: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Scheduled future Bubble Up entries. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["BubbleUpViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Bubble Up view lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_drafts: {
        parameters: {
            query?: {
                cursor?: string | null;
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Drafts mail view. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MailViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description JMAP mail view lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_feed: {
        parameters: {
            query?: {
                cursor?: string | null;
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Feed mail view. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MailViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description JMAP mail view lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_imbox: {
        parameters: {
            query?: {
                cursor?: string | null;
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Imbox mail view. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MailViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description JMAP mail view lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_papertrail: {
        parameters: {
            query?: {
                cursor?: string | null;
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Paper Trail mail view. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MailViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description JMAP mail view lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_reply_later: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Threads in the Reply Later pile. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PileViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Pile lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_screener: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Pending screener senders. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ScreenerViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Screener lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_denied_senders: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Denied screener senders. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DeniedSendersResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Denied sender lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_search: {
        parameters: {
            query?: {
                q?: string | null;
                scope?: string | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Unified mail/contact-note search. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SearchResponse"];
                };
            };
            /** @description Invalid search query. */
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
            /** @description Search lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_set_aside: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Threads in the Set Aside pile. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PileViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Pile lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_trash: {
        parameters: {
            query?: {
                cursor?: string | null;
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Trash mail view. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MailViewResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description JMAP mail view lookup failed. */
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
