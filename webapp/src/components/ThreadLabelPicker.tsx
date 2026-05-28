import { useMemo, useState } from 'react';
import type { HailApiClient, LabelResponse } from '../api/client';
import {
  useAssignLabelNameToThreadMutation,
  useAssignLabelToThreadMutation,
  useLabels,
  useRemoveLabelFromThreadMutation,
} from '../api/query';
import { Plus, Tags } from './icons';
import { Button } from './ui/button';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from './ui/command';
import {
  Popover,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from './ui/popover';

interface ThreadLabelPickerProps {
  threadId: string;
  assignedLabels: LabelResponse[];
  client: HailApiClient;
}

function normalizeLabelName(name: string) {
  return name
    .split('/')
    .map((segment) => segment.trim().replace(/\s+/g, ' '))
    .join('/')
    .toLocaleLowerCase();
}

function labelLeafText(label: LabelResponse) {
  const leaf = label.leaf_name.trim();
  if (leaf.length > 0) {
    return leaf;
  }
  return label.path_segments.at(-1)?.trim() || label.name;
}

function sortLabels(labels: LabelResponse[]) {
  return [...labels].sort((left, right) =>
    left.name.localeCompare(right.name, undefined, { sensitivity: 'base' }),
  );
}

export function ThreadLabelPicker({
  threadId,
  assignedLabels,
  client,
}: ThreadLabelPickerProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const labelsQuery = useLabels(client, { enabled: open });
  const assignLabel = useAssignLabelToThreadMutation(client);
  const assignLabelName = useAssignLabelNameToThreadMutation(client);
  const removeLabel = useRemoveLabelFromThreadMutation(client);
  const [error, setError] = useState<string | null>(null);

  const assignedIds = useMemo(
    () => new Set(assignedLabels.map((label) => label.id)),
    [assignedLabels],
  );
  const allLabels = sortLabels(labelsQuery.data?.labels ?? assignedLabels);
  const trimmedSearch = search.trim();
  const normalizedSearch = normalizeLabelName(trimmedSearch);
  const exactMatch = allLabels.some(
    (label) => normalizeLabelName(label.name) === normalizedSearch,
  );
  const canCreate = trimmedSearch.length > 0 && !exactMatch;
  const busy = assignLabel.isPending || assignLabelName.isPending || removeLabel.isPending;

  async function toggleLabel(label: LabelResponse) {
    setError(null);
    try {
      if (assignedIds.has(label.id)) {
        await removeLabel.mutateAsync({ threadId, labelId: label.id });
      } else {
        await assignLabel.mutateAsync({ threadId, labelId: label.id });
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Label update failed');
    }
  }

  async function createAndAssignLabel() {
    if (!canCreate || busy) {
      return;
    }
    setError(null);
    try {
      await assignLabelName.mutateAsync({
        threadId,
        request: { label_name: trimmedSearch },
      });
      setSearch('');
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Label update failed');
    }
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button type="button" variant="outline" size="xs" aria-label="Manage thread labels">
          <Tags data-icon="inline-start" />
          Labels
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-2">
        <PopoverHeader>
          <PopoverTitle role="heading" aria-level={2}>Manage labels</PopoverTitle>
        </PopoverHeader>
        <p className="px-2 pb-2 text-xs leading-5 text-muted-foreground">
          Check every label that should stay on this thread. Adding one label keeps
          the others assigned.
        </p>
        <Command shouldFilter>
          <CommandInput
            value={search}
            onValueChange={setSearch}
            placeholder="Search or create label…"
            disabled={busy}
          />
          <CommandList>
            <CommandEmpty>No labels found.</CommandEmpty>
            <CommandGroup heading="Add or remove labels">
              {allLabels.map((label) => {
                const checked = assignedIds.has(label.id);
                return (
                  <CommandItem
                    key={label.id}
                    value={label.name}
                    data-checked={checked}
                    aria-checked={checked}
                    aria-label={`${checked ? 'Remove' : 'Add'} label ${label.name}`}
                    disabled={busy}
                    onSelect={() => {
                      void toggleLabel(label);
                    }}
                  >
                    <span className="min-w-0 flex-1 truncate" title={label.name}>
                      {labelLeafText(label)}
                    </span>
                    {label.name !== labelLeafText(label) ? (
                      <CommandShortcut className="max-w-28 truncate normal-case tracking-normal">
                        {label.name}
                      </CommandShortcut>
                    ) : null}
                  </CommandItem>
                );
              })}
            </CommandGroup>
            {canCreate ? (
              <>
                <CommandSeparator />
                <CommandGroup>
                  <CommandItem
                    value={`create-${trimmedSearch}`}
                    disabled={busy}
                    onSelect={() => {
                      void createAndAssignLabel();
                    }}
                  >
                    <Plus data-icon="inline-start" />
                    Create “{trimmedSearch}”
                  </CommandItem>
                </CommandGroup>
              </>
            ) : null}
          </CommandList>
        </Command>
        {error ? (
          <p role="alert" className="px-2 text-xs text-destructive">
            {error}
          </p>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}
