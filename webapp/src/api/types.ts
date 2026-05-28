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
    "/api/attachments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_attachments"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/attachments/{blob_id}/download": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["download_attachment"];
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
    "/api/invite/{token}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_invite"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/invite/{token}/accept": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["accept_invite"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/labels": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_labels"];
        put?: never;
        post: operations["create_label"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/labels/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post?: never;
        delete: operations["delete_label"];
        options?: never;
        head?: never;
        patch: operations["rename_label"];
        trace?: never;
    };
    "/api/labels/{id}/threads": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["label_threads"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/provider-accounts/gmail/callback": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["gmail_callback"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/provider-accounts/gmail/connect": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["connect_gmail"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/provider-accounts/sync-status": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_provider_sync_statuses"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/provider-accounts/{id}/disconnect": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["disconnect_provider_account"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/provider-accounts/{id}/sync": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["trigger_provider_sync"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/scheduled-sends": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_scheduled_sends"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/scheduled-sends/{scheduled_send_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_scheduled_send"];
        put?: never;
        post?: never;
        delete: operations["cancel_scheduled_send"];
        options?: never;
        head?: never;
        patch?: never;
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
    "/api/speakeasy": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_speakeasy"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/speakeasy/rotate": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["rotate_speakeasy"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/labels": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["assign_label_to_threads"];
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
    "/api/threads/{thread_id}/labels": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["assign_label_name_to_thread"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/threads/{thread_id}/labels/{label_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["assign_label_to_thread"];
        delete: operations["remove_label_from_thread"];
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
    "/api/threads/{thread_id}/not-spam": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["not_spam_thread"];
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
    "/api/threads/{thread_id}/spam": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["spam_thread"];
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
    "/api/views/archive": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_archive"];
        put?: never;
        post?: never;
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
    "/api/views/counts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_view_counts"];
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
    "/api/views/imbox/sectioned": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_imbox_sectioned"];
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
    "/api/views/screener/allowed": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_allowed_senders"];
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
    "/api/views/spam": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_spam"];
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
    "/api/workflows": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_workflows"];
        put?: never;
        post: operations["create_workflow"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/workflows/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_workflow"];
        put: operations["update_workflow"];
        post?: never;
        delete: operations["delete_workflow"];
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
        AcceptInviteRequest: {
            password: string;
        };
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
        AllowedSender: {
            classify_as: components["schemas"]["MailClassification"];
            /** Format: date-time */
            decided_at?: string | null;
            /** Format: date-time */
            first_seen_at: string;
            sender_address: string;
        };
        AllowedSendersResponse: {
            allowed: components["schemas"]["AllowedSender"][];
        };
        AssignLabelNameRequest: {
            label_name: string;
        };
        AttachmentContext: {
            email_id: string;
            from: string;
            preview: string;
            /** Format: date-time */
            received_at?: string | null;
            subject: string;
            thread_id: string;
        };
        AttachmentItem: {
            blob_id: string;
            context: components["schemas"]["AttachmentContext"];
            download_url: string;
            name: string;
            size: number;
            type: string;
        };
        AttachmentsResponse: {
            items: components["schemas"]["AttachmentItem"][];
        };
        BatchAssignLabelRequest: {
            /** Format: int64 */
            label_id?: number | null;
            label_name?: string | null;
            thread_ids: string[];
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
            from: string;
            preview: string;
            subject: string;
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
        ClassifyRequest: {
            to: components["schemas"]["MailClassification"];
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
        CreateLabelRequest: {
            color?: string | null;
            name: string;
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
            classify_as?: null | components["schemas"]["MailClassification"];
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
        GmailConnectResponse: {
            authorization_url: string;
            scopes: string[];
        };
        ImboxSectionedResponse: {
            bubbled_up: components["schemas"]["MailViewItem"][];
            new_count: number;
            new_for_you: components["schemas"]["MailViewItem"][];
            previously_seen: components["schemas"]["MailViewItem"][];
            previously_seen_total: number;
        };
        InviteAcceptResponse: {
            user: components["schemas"]["UserView"];
        };
        InvitePreviewResponse: {
            display_name?: string | null;
            email: string;
            /** Format: date-time */
            expires_at: string;
        };
        LabelItemResponse: {
            label: components["schemas"]["LabelResponse"];
        };
        LabelListResponse: {
            labels: components["schemas"]["LabelResponse"][];
        };
        LabelResponse: {
            color?: string | null;
            /** Format: int64 */
            id: number;
            leaf_name: string;
            name: string;
            path_segments: string[];
            source: components["schemas"]["LabelSourceResponse"];
            /** Format: int64 */
            thread_count: number;
        };
        /** @enum {string} */
        LabelSourceResponse: "manual" | "gmail";
        LabelThreadItem: {
            from: string;
            labels: components["schemas"]["LabelResponse"][];
            preview: string;
            subject: string;
            thread_id: string;
        };
        LabelThreadsResponse: {
            items: components["schemas"]["LabelThreadItem"][];
            label: components["schemas"]["LabelResponse"];
            next_cursor?: string | null;
        };
        /**
         * @description Canonical hail-owned routing classification for incoming mail.
         *
         *     These values are stored in sidecar rule rows as lowercase strings and are
         *     represented in JMAP as hail-owned keywords.
         * @enum {string}
         */
        MailClassification: "imbox" | "feed" | "papertrail";
        /** @enum {string} */
        MailViewClassification: "imbox" | "feed" | "papertrail" | "drafts" | "trash" | "spam" | "archive";
        MailViewItem: {
            bcc: string[];
            cc: string[];
            classification: components["schemas"]["MailViewClassification"];
            email_id: string;
            from: string;
            has_notes: boolean;
            labels: components["schemas"]["LabelResponse"][];
            preview: string;
            /** Format: date-time */
            received_at?: string | null;
            subject: string;
            thread_id: string;
            to: string[];
            unread: boolean;
            feed_html?: string | null;
            feed_blocked_trackers?: components["schemas"]["FeedBlockedTrackerResponse"][] | null;
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
        ProviderAccountResponse: {
            /** Format: date-time */
            cached_access_token_expires_at?: string | null;
            display_email?: string | null;
            granted_scopes: string[];
            /** Format: int64 */
            id: number;
            last_profile_history_id?: string | null;
            provider_account_id: string;
            provider_email: string;
            provider_kind: string;
            sync_status: string;
        };
        ProviderSyncEventSummary: {
            /** Format: date-time */
            created_at: string;
            event_type: string;
            result_status: string;
            safe_error_class?: string | null;
            safe_error_message?: string | null;
        };
        ProviderSyncStatusListResponse: {
            accounts: components["schemas"]["ProviderSyncStatusResponse"][];
        };
        ProviderSyncStatusResponse: {
            display_email?: string | null;
            /** Format: int64 */
            id: number;
            last_error_class?: string | null;
            last_error_event?: null | components["schemas"]["ProviderSyncEventSummary"];
            last_error_message?: string | null;
            last_profile_history_id?: string | null;
            /** Format: date-time */
            last_sync_attempted_at?: string | null;
            last_sync_event?: null | components["schemas"]["ProviderSyncEventSummary"];
            /** Format: date-time */
            last_sync_succeeded_at?: string | null;
            /** Format: date-time */
            next_sync_after?: string | null;
            /** Format: date-time */
            profile_synced_at?: string | null;
            provider_account_id: string;
            provider_email: string;
            provider_kind: string;
            /** Format: int64 */
            sync_backoff_secs?: number | null;
            sync_status: string;
        };
        ProviderSyncTriggerResponse: {
            account: components["schemas"]["ProviderSyncStatusResponse"];
        };
        PutNoteRequest: {
            markdown: string;
        };
        RenameLabelRequest: {
            name: string;
        };
        ReplyPayload: {
            attachments?: unknown[] | null;
            body_markdown: string;
            /** Format: date-time */
            send_at?: string | null;
        };
        RotateSpeakeasyRequest: {
            /**
             * @description Optional explicit acknowledgement that this invalidates the previous
             *     phrase immediately. Omitted/false is still accepted for initial API
             *     clients; the field exists so the SPA can make the warning explicit.
             */
            acknowledge_bypass_secret?: boolean;
        };
        ScheduledSendResponse: {
            /** Format: date-time */
            claimed_at?: string | null;
            /** Format: date-time */
            created_at: string;
            draft_email_id: string;
            error?: string | null;
            /** Format: int64 */
            id: number;
            /** Format: date-time */
            send_at: string;
            /** Format: date-time */
            sent_at?: string | null;
            status: string;
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
            labels: components["schemas"]["LabelResponse"][];
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
        SpeakeasyResponse: {
            speakeasy: components["schemas"]["SpeakeasyState"];
        };
        SpeakeasyState: {
            /** Format: date-time */
            generated_at: string;
            /** Format: date-time */
            manually_rotated_at?: string | null;
            /**
             * @description Raw current bypass passphrase. This is intentionally returned only to
             *     the authenticated owner so the UI can display/share it.
             */
            passphrase: string;
            /** @description UTC month this phrase is current for, formatted YYYY-MM. */
            period: string;
            /** Format: date-time */
            rotates_at: string;
        };
        /** @enum {string} */
        StalwartStatus: "connected" | "unreachable";
        ThreadMessageResponse: {
            blocked_trackers: components["schemas"]["BlockedTrackerResponse"][];
            email_id: string;
            from: components["schemas"]["Participant"][];
            html: string;
            html_with_remote_images: string;
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
            labels: components["schemas"]["LabelResponse"][];
            messages: components["schemas"]["ThreadMessageResponse"][];
            notes: components["schemas"]["ThreadNoteResponse"][];
            participants: components["schemas"]["Participant"][];
            subject: string;
            thread_id: string;
        };
        UndoDenyRequest: {
            classify_as?: string | null;
        };
        UndoDenyResponse: {
            classify_as: components["schemas"]["MailClassification"];
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
        /**
         * @description Public JSON representation of a user. Mirrors the v1 schema in
         *     design.md §6.2. `jmap_account_id` and `created_at` are intentionally
         *     NOT exposed — those are server-side bookkeeping.
         */
        UserView: {
            display_name?: string | null;
            email: string;
            /** Format: int64 */
            id: number;
            is_admin: boolean;
        };
        ViewCountsResponse: {
            bubble_up: number;
            drafts: number;
            feed_unread: number;
            imbox_new: number;
            papertrail_unread: number;
            reply_later: number;
            scheduled: number;
            screener_pending: number;
            set_aside: number;
            spam: number;
            trash: number;
        };
        WorkflowAction: {
            add_label?: string | null;
            auto_reply?: string | null;
            classify_as?: null | components["schemas"]["MailClassification"];
        };
        WorkflowCondition: {
            field: components["schemas"]["WorkflowConditionField"];
            op: components["schemas"]["WorkflowConditionOp"];
            value: string;
        };
        /** @enum {string} */
        WorkflowConditionField: "from" | "to" | "cc" | "subject";
        /** @enum {string} */
        WorkflowConditionOp: "contains" | "equals";
        WorkflowRule: {
            action: components["schemas"]["WorkflowAction"];
            conditions: components["schemas"]["WorkflowCondition"][];
            /** Format: date-time */
            created_at: string;
            enabled: boolean;
            /** Format: int64 */
            id: number;
            name: string;
            /** Format: date-time */
            updated_at: string;
        };
        WorkflowRuleListResponse: {
            rules: components["schemas"]["WorkflowRule"][];
        };
        WorkflowRulePayload: {
            action: components["schemas"]["WorkflowAction"];
            conditions: components["schemas"]["WorkflowCondition"][];
            enabled?: boolean;
            name: string;
        };
        WorkflowRuleResponse: {
            rule: components["schemas"]["WorkflowRule"];
        };
        FeedBlockedTrackerResponse: {
            src: string;
            reason: string;
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
    list_attachments: {
        parameters: {
            query?: {
                /** @description Maximum number of messages-with-attachments to inspect. */
                limit?: number | null;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Recent attachments with thread/message context. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AttachmentsResponse"];
                };
            };
            /** @description Invalid query. */
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
            /** @description Attachment listing failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    download_attachment: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP blob id to download. */
                blob_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Attachment bytes. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Invalid blob id. */
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
            /** @description Blob not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Attachment download failed. */
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
    get_invite: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Opaque invite token. */
                token: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Invite can be accepted. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InvitePreviewResponse"];
                };
            };
            /** @description Invite is missing, expired, or already used. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    accept_invite: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Opaque invite token. */
                token: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AcceptInviteRequest"];
            };
        };
        responses: {
            /** @description Invite accepted; session cookie has been set. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["InviteAcceptResponse"];
                };
            };
            /** @description Password failed validation. */
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Invite is missing, expired, or already used. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Upstream user provisioning failed. */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    list_labels: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Labels for the current user. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LabelListResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    create_label: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateLabelRequest"];
            };
        };
        responses: {
            /** @description Label created. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LabelItemResponse"];
                };
            };
            /** @description Invalid label payload or duplicate name. */
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
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label create failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    delete_label: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Label id. */
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Label deleted; thread label assignments cascade. */
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
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label delete failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    rename_label: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Label id. */
                id: number;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RenameLabelRequest"];
            };
        };
        responses: {
            /** @description Label renamed. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LabelItemResponse"];
                };
            };
            /** @description Invalid label payload or duplicate name. */
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
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label rename failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    label_threads: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                /** @description Label id. */
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Threads assigned to this label for the current user. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LabelThreadsResponse"];
                };
            };
            /** @description Invalid cursor. */
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
            /** @description Label not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label thread lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    gmail_callback: {
        parameters: {
            query?: {
                state?: string;
                code?: string;
                error?: string;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Redirects to provider accounts SPA route after Gmail OAuth callback. */
            303: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    connect_gmail: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Gmail OAuth authorization URL. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["GmailConnectResponse"];
                };
            };
        };
    };
    list_provider_sync_statuses: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Connected Gmail provider account sync statuses. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProviderSyncStatusListResponse"];
                };
            };
        };
    };
    disconnect_provider_account: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Provider account disconnected. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProviderAccountResponse"];
                };
            };
        };
    };
    trigger_provider_sync: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Provider account marked due for safe background sync. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProviderSyncTriggerResponse"];
                };
            };
        };
    };
    list_scheduled_sends: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Scheduled sends for the current user. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ScheduledSendResponse"][];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Scheduled send list failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_scheduled_send: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Scheduled send id. */
                scheduled_send_id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Scheduled send detail for the current user. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ScheduledSendResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Scheduled send not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Scheduled send lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    cancel_scheduled_send: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Scheduled send id. */
                scheduled_send_id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Scheduled send cancelled or already cancelled. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ScheduledSendResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Scheduled send not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Scheduled send is not cancellable. */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Scheduled send cancel failed. */
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
                /** @description Normalized sender address to approve and route out of screened-out mail. */
                address: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["UndoDenyRequest"];
            };
        };
        responses: {
            /** @description Denied sender approved and routed. */
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
    get_speakeasy: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Current Speakeasy bypass passphrase and rotation metadata. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SpeakeasyResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Speakeasy lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    rotate_speakeasy: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RotateSpeakeasyRequest"];
            };
        };
        responses: {
            /** @description Speakeasy passphrase rotated immediately. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SpeakeasyResponse"];
                };
            };
            /** @description Invalid JSON payload. */
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
            /** @description CSRF header missing. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Speakeasy rotation failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    assign_label_to_threads: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["BatchAssignLabelRequest"];
            };
        };
        responses: {
            /** @description Existing normalized label reused or a manual label created, then assigned idempotently to every selected thread. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LabelItemResponse"];
                };
            };
            /** @description Invalid JSON, payload shape, thread id, or label name. */
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
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label id not found for the current user. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Batch label assignment failed. */
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
    assign_label_name_to_thread: {
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
                "application/json": components["schemas"]["AssignLabelNameRequest"];
            };
        };
        responses: {
            /** @description Existing normalized label reused or a manual label created, then assigned idempotently. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LabelItemResponse"];
                };
            };
            /** @description Invalid thread id, JSON, or label name. */
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
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Inline label assignment failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    assign_label_to_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
                /** @description Label id. */
                label_id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Label assigned to the thread. Duplicate assignment is idempotent. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LabelItemResponse"];
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
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label not found for the current user. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label assignment failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    remove_label_from_thread: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description JMAP thread id. */
                thread_id: string;
                /** @description Label id. */
                label_id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Label assignment removed. Removing a non-assigned current-user label is idempotent. */
            204: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
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
            /** @description Missing CSRF header. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label not found for the current user. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Label assignment removal failed. */
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
    not_spam_thread: {
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
            /** @description Thread marked as not spam and restored to Imbox. */
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
            /** @description Thread not-spam failed. */
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
    spam_thread: {
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
            /** @description Thread marked as spam. */
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
            /** @description Thread spam failed. */
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
    get_archive: {
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
            /** @description Archive mail view. */
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
    get_view_counts: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Cheap sidebar navigation counts for the current user. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ViewCountsResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description View count lookup failed. */
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
    get_imbox_sectioned: {
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
            /** @description Imbox mail view partitioned into Bubble Up, new, and seen sections. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ImboxSectionedResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Imbox sectioned view lookup failed. */
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
    get_allowed_senders: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Allowed screener senders and routing classifications. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AllowedSendersResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Allowed sender lookup failed. */
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
                q?: string;
                scope?: string;
                /** @example imbox */
                mailbox?: string;
                /** @example 12 */
                label_id?: number;
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
    get_spam: {
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
            /** @description Spam mail view. */
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
    list_workflows: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Workflow rules for the current user. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkflowRuleListResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Workflow rule lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    create_workflow: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["WorkflowRulePayload"];
            };
        };
        responses: {
            /** @description Workflow rule created. */
            201: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkflowRuleResponse"];
                };
            };
            /** @description Invalid workflow rule payload. */
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
            /** @description Workflow rule create failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    get_workflow: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Workflow rule id. */
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Workflow rule detail. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkflowRuleResponse"];
                };
            };
            /** @description Missing or invalid session. */
            401: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Workflow rule not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Workflow rule lookup failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    update_workflow: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Workflow rule id. */
                id: number;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["WorkflowRulePayload"];
            };
        };
        responses: {
            /** @description Workflow rule updated. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WorkflowRuleResponse"];
                };
            };
            /** @description Invalid workflow rule payload. */
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
            /** @description Workflow rule not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Workflow rule update failed. */
            500: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
        };
    };
    delete_workflow: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                /** @description Workflow rule id. */
                id: number;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Workflow rule deleted. */
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
            /** @description Workflow rule not found. */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content?: never;
            };
            /** @description Workflow rule delete failed. */
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
