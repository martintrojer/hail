import { useMemo, useState, type FormEvent } from 'react';
import type {
  HailApiClient,
  MailClassification,
  WorkflowAction,
  WorkflowCondition,
  WorkflowConditionField,
  WorkflowConditionOp,
  WorkflowRule,
  WorkflowRulePayload,
} from '../api/client';
import {
  useCreateWorkflowMutation,
  useDeleteWorkflowMutation,
  useUpdateWorkflowMutation,
  useWorkflows,
} from '../api/query';
import { ErrorState } from '../components/ErrorState';
import { LoadingState } from '../components/LoadingState';
import { StateCard } from '../components/StateCard';
import { AppShell } from '../layout/AppShell';
import { pillButtonClass } from '../lib/buttonStyles';
import { formatDateTime } from '../lib/dates';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';

interface WorkflowsPageProps {
  client?: HailApiClient;
}
interface ConditionDraft {
  id: string;
  field: WorkflowConditionField;
  op: WorkflowConditionOp;
  value: string;
}
interface WorkflowFormState {
  id: number | null;
  name: string;
  enabled: boolean;
  conditions: ConditionDraft[];
  classifyAs: MailClassification | '';
  addLabel: string;
  autoReply: string;
}

const fieldLabels: Record<WorkflowConditionField, string> = {
  from: 'From',
  to: 'To',
  cc: 'Cc',
  subject: 'Subject',
};
const opLabels: Record<WorkflowConditionOp, string> = {
  contains: 'contains',
  equals: 'equals',
};
const classificationLabels: Record<MailClassification, string> = {
  imbox: 'Imbox',
  feed: 'The Feed',
  papertrail: 'Paper Trail',
};
const conditionFields = Object.keys(fieldLabels) as WorkflowConditionField[];
const conditionOps = Object.keys(opLabels) as WorkflowConditionOp[];
const classifications = Object.keys(
  classificationLabels,
) as MailClassification[];
let nextConditionId = 0;

function newCondition(condition?: WorkflowCondition): ConditionDraft {
  nextConditionId += 1;
  return {
    id: `condition-${nextConditionId}`,
    field: condition?.field ?? 'from',
    op: condition?.op ?? 'contains',
    value: condition?.value ?? '',
  };
}

function emptyForm(): WorkflowFormState {
  return {
    id: null,
    name: '',
    enabled: true,
    conditions: [newCondition()],
    classifyAs: '',
    addLabel: '',
    autoReply: '',
  };
}

function formFromRule(rule: WorkflowRule): WorkflowFormState {
  return {
    id: rule.id,
    name: rule.name,
    enabled: rule.enabled,
    conditions: rule.conditions.map((condition) => newCondition(condition)),
    classifyAs: rule.action.classify_as ?? '',
    addLabel: rule.action.add_label ?? '',
    autoReply: rule.action.auto_reply ?? '',
  };
}

function payloadFromForm(form: WorkflowFormState): WorkflowRulePayload {
  return {
    name: form.name.trim(),
    enabled: form.enabled,
    conditions: form.conditions.map(({ field, op, value }) => ({
      field,
      op,
      value: value.trim(),
    })),
    action: {
      classify_as: form.classifyAs || null,
      add_label: form.addLabel.trim() || null,
      auto_reply: form.autoReply.trim() || null,
    },
  };
}

function describeCondition(condition: WorkflowCondition) {
  return `${fieldLabels[condition.field]} ${opLabels[condition.op]} “${condition.value}”`;
}

function actionSummary(action: WorkflowAction) {
  const parts: string[] = [];
  if (action.classify_as)
    parts.push(`route to ${classificationLabels[action.classify_as]}`);
  if (action.add_label) parts.push(`label “${action.add_label}”`);
  if (action.auto_reply) parts.push('send auto-reply');
  return parts.length > 0 ? parts.join(', ') : 'No action';
}

function WorkflowsIntro({ count }: { count: number }) {
  return (
    <section className="rounded-2xl border border-border-subtle bg-bg-surface p-5 shadow-sm shadow-ink-primary/5">
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-ink-tertiary">
        Workflows
      </p>
      <h2 className="mt-2 text-xl font-semibold text-ink-primary">
        Teach hail how to sort recurring mail.
      </h2>
      <p className="mt-2 max-w-3xl text-sm leading-6 text-ink-secondary">
        Build simple mail rules from header conditions. When a message matches,
        hail can route it to a mailbox, apply a label, or prepare an auto-reply.
      </p>
      <p className="mt-4 text-sm font-medium text-ink-primary">
        {count} {count === 1 ? 'rule' : 'rules'} configured
      </p>
    </section>
  );
}

function WorkflowRuleCard({
  rule,
  selected,
  deleting,
  onEdit,
  onDelete,
}: {
  rule: WorkflowRule;
  selected: boolean;
  deleting: boolean;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <article
      className={`rounded-2xl border bg-bg-surface p-4 shadow-sm shadow-ink-primary/5 transition ${selected ? 'border-accent-blue' : 'border-border-subtle hover:border-border-menu'}`}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-lg font-semibold text-ink-primary">
              {rule.name}
            </h3>
            <span
              className={`rounded-full px-2.5 py-1 text-xs font-semibold ${rule.enabled ? 'bg-bg-selected text-accent-blue' : 'border border-border-menu text-ink-tertiary'}`}
            >
              {rule.enabled ? 'On' : 'Off'}
            </span>
          </div>
          <p className="mt-2 text-sm text-ink-secondary">
            If {rule.conditions.map(describeCondition).join(' and ')}
          </p>
          <p className="mt-1 text-sm font-medium text-ink-primary">
            Then {actionSummary(rule.action)}
          </p>
          <p className="mt-3 text-xs text-ink-tertiary">
            Updated {formatDateTime(rule.updated_at)}
          </p>
        </div>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            onClick={onEdit}
            className={pillButtonClass('outline', 'md')}
          >
            Edit
          </button>
          <button
            type="button"
            onClick={onDelete}
            disabled={deleting}
            className={pillButtonClass('danger', 'md')}
          >
            {deleting ? 'Deleting…' : 'Delete'}
          </button>
        </div>
      </div>
    </article>
  );
}

function WorkflowForm({
  form,
  setForm,
  saving,
  error,
  onSubmit,
  onReset,
}: {
  form: WorkflowFormState;
  setForm: (form: WorkflowFormState) => void;
  saving: boolean;
  error: Error | null;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  onReset: () => void;
}) {
  const editing = form.id !== null;
  const hasAction = Boolean(
    form.classifyAs || form.addLabel.trim() || form.autoReply.trim(),
  );

  function updateCondition(
    id: string,
    patch: Partial<Omit<ConditionDraft, 'id'>>,
  ) {
    setForm({
      ...form,
      conditions: form.conditions.map((condition) =>
        condition.id === id ? { ...condition, ...patch } : condition,
      ),
    });
  }

  function removeCondition(id: string) {
    if (form.conditions.length === 1) return;
    setForm({
      ...form,
      conditions: form.conditions.filter((condition) => condition.id !== id),
    });
  }

  return (
    <form
      onSubmit={onSubmit}
      className="rounded-2xl border border-border-subtle bg-bg-surface p-5 shadow-sm shadow-ink-primary/5"
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-ink-tertiary">
            Rule builder
          </p>
          <h2 className="mt-2 text-xl font-semibold text-ink-primary">
            {editing ? 'Edit workflow' : 'Create workflow'}
          </h2>
        </div>
        {editing ? (
          <button
            type="button"
            onClick={onReset}
            className={pillButtonClass('ghost', 'md')}
          >
            New rule
          </button>
        ) : null}
      </div>

      <div className="mt-5 grid gap-4">
        <label
          className="block text-sm font-medium text-ink-secondary"
          htmlFor="workflow-name"
        >
          Name
          <input
            id="workflow-name"
            value={form.name}
            onChange={(event) => setForm({ ...form, name: event.target.value })}
            required
            placeholder="Receipts to Paper Trail"
            className="mt-2 w-full rounded-lg border border-border-menu bg-bg-page px-3 py-2 text-ink-primary outline-none ring-accent-blue transition placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2"
          />
        </label>

        <label className="flex items-center gap-3 text-sm font-medium text-ink-secondary">
          <input
            type="checkbox"
            checked={form.enabled}
            onChange={(event) =>
              setForm({ ...form, enabled: event.target.checked })
            }
            className="h-4 w-4 rounded border-border-menu accent-accent-blue"
          />
          Enabled
        </label>

        <section className="rounded-xl border border-border-hairline bg-bg-page/60 p-4">
          <div className="flex items-center justify-between gap-3">
            <h3 className="font-semibold text-ink-primary">Conditions</h3>
            <button
              type="button"
              onClick={() =>
                setForm({
                  ...form,
                  conditions: [...form.conditions, newCondition()],
                })
              }
              className={pillButtonClass('outline', 'sm')}
            >
              Add condition
            </button>
          </div>
          <div className="mt-4 space-y-3">
            {form.conditions.map((condition, index) => (
              <div
                key={condition.id}
                className="grid gap-2 sm:grid-cols-[1fr_1fr_minmax(0,2fr)_auto] sm:items-end"
              >
                <label className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-tertiary">
                  Field
                  <select
                    value={condition.field}
                    onChange={(event) =>
                      updateCondition(condition.id, {
                        field: event.target.value as WorkflowConditionField,
                      })
                    }
                    className="mt-1 w-full rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-sm text-ink-primary outline-none focus:border-accent-blue focus:ring-2 focus:ring-accent-blue"
                    aria-label={`Condition ${index + 1} field`}
                  >
                    {conditionFields.map((field) => (
                      <option key={field} value={field}>
                        {fieldLabels[field]}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-tertiary">
                  Match
                  <select
                    value={condition.op}
                    onChange={(event) =>
                      updateCondition(condition.id, {
                        op: event.target.value as WorkflowConditionOp,
                      })
                    }
                    className="mt-1 w-full rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-sm text-ink-primary outline-none focus:border-accent-blue focus:ring-2 focus:ring-accent-blue"
                    aria-label={`Condition ${index + 1} operator`}
                  >
                    {conditionOps.map((op) => (
                      <option key={op} value={op}>
                        {opLabels[op]}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="text-xs font-semibold uppercase tracking-[0.12em] text-ink-tertiary">
                  Value
                  <input
                    value={condition.value}
                    onChange={(event) =>
                      updateCondition(condition.id, {
                        value: event.target.value,
                      })
                    }
                    required
                    placeholder="billing@example.com"
                    className="mt-1 w-full rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-sm text-ink-primary outline-none placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2 focus:ring-accent-blue"
                    aria-label={`Condition ${index + 1} value`}
                  />
                </label>
                <button
                  type="button"
                  onClick={() => removeCondition(condition.id)}
                  disabled={form.conditions.length === 1}
                  className={pillButtonClass('ghost', 'sm')}
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
        </section>

        <section className="rounded-xl border border-border-hairline bg-bg-page/60 p-4">
          <h3 className="font-semibold text-ink-primary">Actions</h3>
          <div className="mt-4 grid gap-4">
            <label
              className="block text-sm font-medium text-ink-secondary"
              htmlFor="workflow-classify"
            >
              Route to
              <select
                id="workflow-classify"
                value={form.classifyAs}
                onChange={(event) =>
                  setForm({
                    ...form,
                    classifyAs: event.target.value as MailClassification | '',
                  })
                }
                className="mt-2 w-full rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-ink-primary outline-none focus:border-accent-blue focus:ring-2 focus:ring-accent-blue"
              >
                <option value="">Do not route</option>
                {classifications.map((classification) => (
                  <option key={classification} value={classification}>
                    {classificationLabels[classification]}
                  </option>
                ))}
              </select>
            </label>
            <label
              className="block text-sm font-medium text-ink-secondary"
              htmlFor="workflow-label"
            >
              Add label
              <input
                id="workflow-label"
                value={form.addLabel}
                onChange={(event) =>
                  setForm({ ...form, addLabel: event.target.value })
                }
                placeholder="Receipts"
                className="mt-2 w-full rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-ink-primary outline-none placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2 focus:ring-accent-blue"
              />
            </label>
            <label
              className="block text-sm font-medium text-ink-secondary"
              htmlFor="workflow-auto-reply"
            >
              Auto-reply text
              <textarea
                id="workflow-auto-reply"
                value={form.autoReply}
                onChange={(event) =>
                  setForm({ ...form, autoReply: event.target.value })
                }
                placeholder="Thanks — I got this and will reply soon."
                rows={4}
                className="mt-2 w-full rounded-lg border border-border-menu bg-bg-surface px-3 py-2 text-ink-primary outline-none placeholder:text-ink-tertiary focus:border-accent-blue focus:ring-2 focus:ring-accent-blue"
              />
            </label>
          </div>
          {!hasAction ? (
            <p className="mt-3 text-sm text-accent-red" role="alert">
              Choose at least one action before saving.
            </p>
          ) : null}
        </section>

        {error ? (
          <p
            role="alert"
            className="rounded-lg border border-red-800 bg-red-950/70 px-3 py-2 text-sm text-red-100"
          >
            {actionErrorMessage(
              error,
              editing ? 'Update workflow' : 'Create workflow',
            )}
          </p>
        ) : null}

        <button
          type="submit"
          disabled={saving || !hasAction}
          className="rounded-lg bg-accent-blue px-4 py-2 font-semibold text-white transition hover:bg-accent-blue-hover disabled:cursor-not-allowed disabled:opacity-60"
        >
          {saving ? 'Saving…' : editing ? 'Save workflow' : 'Create workflow'}
        </button>
      </div>
    </form>
  );
}

export function WorkflowsPage({ client }: WorkflowsPageProps) {
  const query = useWorkflows(client);
  const createWorkflow = useCreateWorkflowMutation(client);
  const updateWorkflow = useUpdateWorkflowMutation(client);
  const deleteWorkflow = useDeleteWorkflowMutation(client);
  const [form, setForm] = useState<WorkflowFormState>(() => emptyForm());
  const [notice, setNotice] = useState<string | null>(null);
  const rules = useMemo(() => query.data?.rules ?? [], [query.data?.rules]);
  const saving = createWorkflow.isPending || updateWorkflow.isPending;
  const saveError = createWorkflow.error ?? updateWorkflow.error;

  function resetForm() {
    setForm(emptyForm());
    setNotice(null);
  }

  function editRule(rule: WorkflowRule) {
    setForm(formFromRule(rule));
    setNotice(null);
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setNotice(null);
    const request = payloadFromForm(form);
    if (form.id === null) {
      createWorkflow.mutate(request, {
        onSuccess: () => {
          resetForm();
          setNotice('Workflow created.');
        },
      });
    } else {
      updateWorkflow.mutate(
        { id: form.id, request },
        {
          onSuccess: ({ rule }) => {
            setForm(formFromRule(rule));
            setNotice('Workflow saved.');
          },
        },
      );
    }
  }

  function deleteRule(rule: WorkflowRule) {
    if (!window.confirm(`Delete workflow “${rule.name}”?`)) return;
    deleteWorkflow.mutate(rule.id, {
      onSuccess: () => {
        if (form.id === rule.id) resetForm();
        setNotice('Workflow deleted.');
      },
    });
  }

  let list;
  if (query.isPending) {
    list = <LoadingState label="Loading workflows" />;
  } else if (query.isError) {
    list = (
      <ErrorState
        message={viewErrorMessage(query.error, 'Workflows')}
        onRetry={() => void query.refetch()}
      />
    );
  } else {
    list = (
      <div className="space-y-4">
        <WorkflowsIntro count={rules.length} />
        {notice ? (
          <p className="rounded-lg border border-border-subtle bg-bg-selected px-3 py-2 text-sm font-medium text-ink-primary">
            {notice}
          </p>
        ) : null}
        {deleteWorkflow.error ? (
          <p
            role="alert"
            className="rounded-lg border border-red-800 bg-red-950/70 px-3 py-2 text-sm text-red-100"
          >
            {actionErrorMessage(deleteWorkflow.error, 'Delete workflow')}
          </p>
        ) : null}
        {rules.length === 0 ? (
          <StateCard
            title="No workflows yet"
            body="Create your first rule to route recurring mail automatically."
          />
        ) : (
          <div className="space-y-3">
            {rules.map((rule) => (
              <WorkflowRuleCard
                key={rule.id}
                rule={rule}
                selected={form.id === rule.id}
                deleting={deleteWorkflow.isPending}
                onEdit={() => editRule(rule)}
                onDelete={() => deleteRule(rule)}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <AppShell
      title="Workflows"
      description="Mail rules for routing, labels, and auto-replies."
      list={
        <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(360px,440px)] xl:items-start">
          <div className="min-w-0">{list}</div>
          <WorkflowForm
            form={form}
            setForm={setForm}
            saving={saving}
            error={saveError}
            onSubmit={submit}
            onReset={resetForm}
          />
        </div>
      }
      wide
    />
  );
}
