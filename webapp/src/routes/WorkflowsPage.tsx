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
import { Alert, AlertDescription } from '../components/ui/alert';
import { Badge } from '../components/ui/badge';
import { Button } from '../components/ui/button';
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import { Checkbox } from '../components/ui/checkbox';
import {
  Field,
  FieldContent,
  FieldGroup,
  FieldLabel,
  FieldSet,
  FieldLegend,
} from '../components/ui/field';
import { Input } from '../components/ui/input';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../components/ui/select';
import { Textarea } from '../components/ui/textarea';
import { AppShell } from '../layout/AppShell';
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
const NO_ROUTE_VALUE = 'none';
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
    <Card>
      <CardHeader>
        <CardDescription>Workflows</CardDescription>
        <CardTitle>Teach hail how to sort recurring mail.</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <p className="max-w-3xl text-sm leading-6 text-muted-foreground">
          Build simple mail rules from header conditions. When a message matches,
          hail can route it to a mailbox, apply a label, or prepare an auto-reply.
        </p>
        <Badge variant="outline" className="w-fit">
          {count} {count === 1 ? 'rule' : 'rules'} configured
        </Badge>
      </CardContent>
    </Card>
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
    <Card className={selected ? 'ring-primary/40' : undefined}>
      <CardHeader>
        <div className="flex flex-wrap items-center gap-2">
          <CardTitle>{rule.name}</CardTitle>
          <Badge variant={rule.enabled ? 'secondary' : 'outline'}>
            {rule.enabled ? 'On' : 'Off'}
          </Badge>
        </div>
        <CardDescription>
          If {rule.conditions.map(describeCondition).join(' and ')}
        </CardDescription>
        <CardAction>
          <div className="flex shrink-0 gap-2">
            <Button type="button" onClick={onEdit} variant="outline" size="sm">
              Edit
            </Button>
            <Button
              type="button"
              onClick={onDelete}
              disabled={deleting}
              variant="destructive"
              size="sm"
            >
              {deleting ? 'Deleting…' : 'Delete'}
            </Button>
          </div>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <p className="text-sm font-medium">
          Then {actionSummary(rule.action)}
        </p>
        <p className="text-xs text-muted-foreground">
          Updated {formatDateTime(rule.updated_at)}
        </p>
      </CardContent>
    </Card>
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
    <Card>
      <form onSubmit={onSubmit}>
        <CardHeader>
          <CardDescription>Rule builder</CardDescription>
          <CardTitle>{editing ? 'Edit workflow' : 'Create workflow'}</CardTitle>
          {editing ? (
            <CardAction>
              <Button type="button" onClick={onReset} variant="ghost" size="sm">
                New rule
              </Button>
            </CardAction>
          ) : null}
        </CardHeader>

        <CardContent>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor="workflow-name">Name</FieldLabel>
              <Input
                id="workflow-name"
                value={form.name}
                onChange={(event) => setForm({ ...form, name: event.target.value })}
                required
                placeholder="Receipts to Paper Trail"
              />
            </Field>

            <Field orientation="horizontal" data-disabled={saving ? true : undefined}>
              <Checkbox
                id="workflow-enabled"
                checked={form.enabled}
                onCheckedChange={(checked) =>
                  setForm({ ...form, enabled: checked === true })
                }
                disabled={saving}
              />
              <FieldContent>
                <FieldLabel htmlFor="workflow-enabled">Enabled</FieldLabel>
              </FieldContent>
            </Field>

            <FieldSet className="rounded-lg border border-border p-4">
              <div className="flex items-center justify-between gap-3">
                <FieldLegend>Conditions</FieldLegend>
                <Button
                  type="button"
                  onClick={() =>
                    setForm({
                      ...form,
                      conditions: [...form.conditions, newCondition()],
                    })
                  }
                  variant="outline"
                  size="sm"
                >
                  Add condition
                </Button>
              </div>
              <div className="flex flex-col gap-3">
                {form.conditions.map((condition, index) => (
                  <div
                    key={condition.id}
                    className="grid gap-2 sm:grid-cols-[1fr_1fr_minmax(0,2fr)_auto] sm:items-end"
                  >
                    <Field>
                      <FieldLabel>Field</FieldLabel>
                      <Select
                        value={condition.field}
                        onValueChange={(value) =>
                          updateCondition(condition.id, {
                            field: value as WorkflowConditionField,
                          })
                        }
                      >
                        <SelectTrigger aria-label={`Condition ${index + 1} field`} className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            {conditionFields.map((field) => (
                              <SelectItem key={field} value={field}>
                                {fieldLabels[field]}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </Field>
                    <Field>
                      <FieldLabel>Match</FieldLabel>
                      <Select
                        value={condition.op}
                        onValueChange={(value) =>
                          updateCondition(condition.id, {
                            op: value as WorkflowConditionOp,
                          })
                        }
                      >
                        <SelectTrigger aria-label={`Condition ${index + 1} operator`} className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            {conditionOps.map((op) => (
                              <SelectItem key={op} value={op}>
                                {opLabels[op]}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </Field>
                    <Field>
                      <FieldLabel htmlFor={`${condition.id}-value`}>Value</FieldLabel>
                      <Input
                        id={`${condition.id}-value`}
                        value={condition.value}
                        onChange={(event) =>
                          updateCondition(condition.id, {
                            value: event.target.value,
                          })
                        }
                        required
                        placeholder="billing@example.com"
                        aria-label={`Condition ${index + 1} value`}
                      />
                    </Field>
                    <Button
                      type="button"
                      onClick={() => removeCondition(condition.id)}
                      disabled={form.conditions.length === 1}
                      variant="ghost"
                      size="sm"
                    >
                      Remove
                    </Button>
                  </div>
                ))}
              </div>
            </FieldSet>

            <FieldSet className="rounded-lg border border-border p-4">
              <FieldLegend>Actions</FieldLegend>
              <FieldGroup>
                <Field>
                  <FieldLabel>Route to</FieldLabel>
                  <Select
                    value={form.classifyAs || NO_ROUTE_VALUE}
                    onValueChange={(value) =>
                      setForm({
                        ...form,
                        classifyAs:
                          value === NO_ROUTE_VALUE
                            ? ''
                            : (value as MailClassification),
                      })
                    }
                  >
                    <SelectTrigger aria-label="Route to" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={NO_ROUTE_VALUE}>Do not route</SelectItem>
                        {classifications.map((classification) => (
                          <SelectItem key={classification} value={classification}>
                            {classificationLabels[classification]}
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </Field>
                <Field>
                  <FieldLabel htmlFor="workflow-label">Add label</FieldLabel>
                  <Input
                    id="workflow-label"
                    value={form.addLabel}
                    onChange={(event) =>
                      setForm({ ...form, addLabel: event.target.value })
                    }
                    placeholder="Receipts"
                  />
                </Field>
                <Field>
                  <FieldLabel htmlFor="workflow-auto-reply">Auto-reply text</FieldLabel>
                  <Textarea
                    id="workflow-auto-reply"
                    value={form.autoReply}
                    onChange={(event) =>
                      setForm({ ...form, autoReply: event.target.value })
                    }
                    placeholder="Thanks — I got this and will reply soon."
                    rows={4}
                  />
                </Field>
              </FieldGroup>
              {!hasAction ? (
                <Alert variant="destructive">
                  <AlertDescription>
                    Choose at least one action before saving.
                  </AlertDescription>
                </Alert>
              ) : null}
            </FieldSet>

            {error ? (
              <Alert variant="destructive">
                <AlertDescription>
                  {actionErrorMessage(
                    error,
                    editing ? 'Update workflow' : 'Create workflow',
                  )}
                </AlertDescription>
              </Alert>
            ) : null}

            <Button type="submit" disabled={saving || !hasAction}>
              {saving ? 'Saving…' : editing ? 'Save workflow' : 'Create workflow'}
            </Button>
          </FieldGroup>
        </CardContent>
      </form>
    </Card>
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
      <div className="flex flex-col gap-4">
        <WorkflowsIntro count={rules.length} />
        {notice ? (
          <Alert>
            <AlertDescription>{notice}</AlertDescription>
          </Alert>
        ) : null}
        {deleteWorkflow.error ? (
          <Alert variant="destructive">
            <AlertDescription>
              {actionErrorMessage(deleteWorkflow.error, 'Delete workflow')}
            </AlertDescription>
          </Alert>
        ) : null}
        {rules.length === 0 ? (
          <StateCard
            title="No workflows yet"
            body="Create your first rule to route recurring mail automatically."
          />
        ) : (
          <div className="flex flex-col gap-3">
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
