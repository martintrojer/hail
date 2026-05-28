import { useEffect, useMemo, useState, type FormEvent } from 'react';
import type { HailApiClient, LabelResponse } from '../api/client';
import {
  useCreateLabelMutation,
  useDeleteLabelMutation,
  useLabels,
  useRenameLabelMutation,
} from '../api/query';
import { useApiClient } from '../api/ApiClientProvider';
import { ErrorState } from '../components/ErrorState';
import { Plus, Tags, Trash2 } from '../components/icons';
import { StateCard } from '../components/StateCard';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '../components/ui/alert-dialog';
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../components/ui/dialog';
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from '../components/ui/field';
import { Input } from '../components/ui/input';
import { Separator } from '../components/ui/separator';
import { Skeleton } from '../components/ui/skeleton';
import { AppShell } from '../layout/AppShell';
import { actionErrorMessage, viewErrorMessage } from '../lib/errorMessages';
import { cn } from '../lib/utils';

interface LabelsManagementPageProps {
  client?: HailApiClient;
}

type LabelFormMode = 'create' | 'rename';

interface LabelFormState {
  mode: LabelFormMode;
  label?: LabelResponse;
}

interface LabelTreeRow {
  label: LabelResponse;
  depth: number;
  displayPath: string;
  parentPath: string;
}

function normalizedSegments(label: LabelResponse) {
  const segments = label.path_segments.length > 0 ? label.path_segments : label.name.split('/');
  return segments.map((segment) => segment.trim()).filter(Boolean);
}

function labelPath(label: LabelResponse) {
  const segments = normalizedSegments(label);
  return segments.length > 0 ? segments.join(' / ') : label.name;
}

function sortLabels(labels: LabelResponse[]) {
  return [...labels].sort((left, right) => {
    const leftSegments = normalizedSegments(left).map((segment) => segment.toLocaleLowerCase());
    const rightSegments = normalizedSegments(right).map((segment) => segment.toLocaleLowerCase());
    const length = Math.max(leftSegments.length, rightSegments.length);

    for (let index = 0; index < length; index += 1) {
      const leftSegment = leftSegments[index];
      const rightSegment = rightSegments[index];

      if (leftSegment === undefined) {
        return -1;
      }
      if (rightSegment === undefined) {
        return 1;
      }
      const segmentOrder = leftSegment.localeCompare(rightSegment);
      if (segmentOrder !== 0) {
        return segmentOrder;
      }
    }

    return left.id - right.id;
  });
}

function treeRows(labels: LabelResponse[]): LabelTreeRow[] {
  return sortLabels(labels).map((label) => {
    const segments = normalizedSegments(label);
    return {
      label,
      depth: Math.max(segments.length - 1, 0),
      displayPath: segments.length > 0 ? segments.join(' / ') : label.name,
      parentPath: segments.slice(0, -1).join(' / '),
    };
  });
}

function labelMutationError(error: Error | null, action: string) {
  return error ? actionErrorMessage(error, action) : null;
}

function LabelFormDialog({
  state,
  onOpenChange,
  client,
}: {
  state: LabelFormState | null;
  onOpenChange: (open: boolean) => void;
  client: HailApiClient;
}) {
  const [name, setName] = useState(state?.label?.name ?? '');
  const isRename = state?.mode === 'rename';
  const createLabel = useCreateLabelMutation(client, {
    onSuccess: () => {
      setName('');
      onOpenChange(false);
    },
  });
  const renameLabel = useRenameLabelMutation(client, {
    onSuccess: () => {
      onOpenChange(false);
    },
  });
  const pending = createLabel.isPending || renameLabel.isPending;
  const trimmedName = name.trim();
  const error = isRename ? renameLabel.error : createLabel.error;

  useEffect(() => {
    setName(state?.label?.name ?? '');
  }, [state]);

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!trimmedName) {
      return;
    }

    if (isRename && state?.label) {
      renameLabel.mutate({ id: state.label.id, request: { name: trimmedName } });
    } else {
      createLabel.mutate({ name: trimmedName });
    }
  }

  return (
    <Dialog open={state !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{isRename ? 'Rename label' : 'Create label'}</DialogTitle>
          <DialogDescription>
            Labels are saved as full paths. Use slashes for nested display, such as Work/Receipts.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={onSubmit} className="flex flex-col gap-4">
          <FieldGroup>
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="label-name">Label name or path</FieldLabel>
              <Input
                id="label-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="Work/Receipts"
                required
                autoFocus
                aria-invalid={Boolean(error)}
              />
              <FieldDescription>
                Empty path segments like Work//Receipts are rejected by the API.
              </FieldDescription>
              <FieldError>{labelMutationError(error, isRename ? 'Rename label' : 'Create label')}</FieldError>
            </Field>
          </FieldGroup>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={pending || trimmedName.length === 0}>
              {pending ? (isRename ? 'Renaming…' : 'Creating…') : (isRename ? 'Rename label' : 'Create label')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function DeleteLabelDialog({
  label,
  onOpenChange,
  client,
}: {
  label: LabelResponse | null;
  onOpenChange: (open: boolean) => void;
  client: HailApiClient;
}) {
  const deleteLabel = useDeleteLabelMutation(client, {
    onSuccess: () => onOpenChange(false),
  });
  const error = labelMutationError(deleteLabel.error, 'Delete label');

  return (
    <AlertDialog open={label !== null} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {label ? labelPath(label) : 'label'}?</AlertDialogTitle>
          <AlertDialogDescription>
            This permanently deletes the label and removes it from every assigned thread. The mail itself is not deleted.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={deleteLabel.isPending}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={deleteLabel.isPending || label === null}
            onClick={(event) => {
              event.preventDefault();
              if (label) {
                deleteLabel.mutate(label.id);
              }
            }}
          >
            {deleteLabel.isPending ? 'Deleting…' : 'Delete label'}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function LabelsLoadingCard() {
  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>Loading labels</CardTitle>
        <CardDescription>Reading label paths from hail.</CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <Skeleton className="h-8 w-full" />
        <Skeleton className="h-8 w-5/6" />
        <Skeleton className="h-8 w-2/3" />
      </CardContent>
    </Card>
  );
}

function LabelRow({
  row,
  onRename,
  onDelete,
}: {
  row: LabelTreeRow;
  onRename: (label: LabelResponse) => void;
  onDelete: (label: LabelResponse) => void;
}) {
  const { label } = row;

  return (
    <li>
      <div
        className="flex items-center gap-3 px-3 py-1.5 hover:bg-muted/50"
        style={{ paddingLeft: `${0.75 + row.depth * 1.25}rem` }}
      >
        <Tags className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-medium text-foreground" title={label.name}>
              {row.displayPath}
            </span>
            <Badge variant={label.source === 'gmail' ? 'outline' : 'secondary'} className="shrink-0">
              {label.source}
            </Badge>
          </div>
          <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
            <span className="truncate">{label.name}</span>
            <span aria-hidden="true">·</span>
            <span>{label.thread_count} {label.thread_count === 1 ? 'thread' : 'threads'}</span>
            {row.parentPath ? <span className="truncate">under {row.parentPath}</span> : null}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <Button type="button" variant="outline" size="sm" onClick={() => onRename(label)}>
            Rename
          </Button>
          <Button
            type="button"
            variant="destructive"
            size="icon-sm"
            aria-label={`Delete ${label.name}`}
            title={`Delete ${label.name}`}
            onClick={() => onDelete(label)}
          >
            <Trash2 />
          </Button>
        </div>
      </div>
    </li>
  );
}

function LabelsList({
  labels,
  onRename,
  onDelete,
}: {
  labels: LabelResponse[];
  onRename: (label: LabelResponse) => void;
  onDelete: (label: LabelResponse) => void;
}) {
  const rows = useMemo(() => treeRows(labels), [labels]);

  if (rows.length === 0) {
    return (
      <StateCard
        title="No labels yet"
        body="Create a label path such as Work/Receipts, then assign it to threads from label actions."
      />
    );
  }

  return (
    <Card size="sm">
      <CardHeader>
        <CardTitle>Label tree</CardTitle>
        <CardDescription>
          Full paths are concrete labels; parent rows are visual grouping only.
        </CardDescription>
      </CardHeader>
      <CardContent className="px-0">
        <Separator />
        <ul aria-label="Labels" data-testid="labels-list">
          {rows.map((row) => (
            <LabelRow
              key={row.label.id}
              row={row}
              onRename={onRename}
              onDelete={onDelete}
            />
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}

export function LabelsManagementPage({ client }: LabelsManagementPageProps) {
  const contextClient = useApiClient();
  const apiClient = client ?? contextClient;
  const labels = useLabels(apiClient);
  const [formState, setFormState] = useState<LabelFormState | null>(null);
  const [deleteLabel, setDeleteLabel] = useState<LabelResponse | null>(null);
  const totalThreads = labels.data?.labels.reduce((sum, label) => sum + label.thread_count, 0) ?? 0;

  let content;
  if (labels.isPending) {
    content = <LabelsLoadingCard />;
  } else if (labels.isError) {
    content = (
      <ErrorState
        message={viewErrorMessage(labels.error, 'Labels')}
        onRetry={() => void labels.refetch()}
      />
    );
  } else {
    content = (
      <LabelsList
        labels={labels.data.labels}
        onRename={(label) => setFormState({ mode: 'rename', label })}
        onDelete={setDeleteLabel}
      />
    );
  }

  return (
    <>
      <AppShell
        title="Labels"
        actions={
          <Button size="sm" onClick={() => setFormState({ mode: 'create' })}>
            <Plus data-icon="inline-start" />
            Create label
          </Button>
        }
        list={
          <div className="flex flex-col gap-4">
            <Card size="sm">
              <CardHeader>
                <CardTitle>Manage labels</CardTitle>
                <CardDescription>
                  Create local thread labels, rename full paths, and delete labels when they are no longer useful.
                </CardDescription>
                <CardAction>
                  <Badge variant="secondary" className={cn(labels.isPending && 'invisible')}>
                    {labels.data?.labels.length ?? 0} labels · {totalThreads} assignments
                  </Badge>
                </CardAction>
              </CardHeader>
            </Card>
            {content}
          </div>
        }
      />
      <LabelFormDialog
        state={formState}
        onOpenChange={(open) => {
          if (!open) {
            setFormState(null);
          }
        }}
        client={apiClient}
      />
      <DeleteLabelDialog
        label={deleteLabel}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteLabel(null);
          }
        }}
        client={apiClient}
      />
    </>
  );
}
