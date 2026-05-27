import type { components } from '../api/types';
import { Badge } from './ui/badge';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

type HydratedLabel = components['schemas']['LabelResponse'];

export interface LabelChipsProps {
  labels?: HydratedLabel[] | null;
  className?: string;
}

function labelLeafText(label: HydratedLabel) {
  const leaf = label.leaf_name.trim();
  if (leaf.length > 0) {
    return leaf;
  }

  const lastSegment = label.path_segments.at(-1)?.trim();
  if (lastSegment && lastSegment.length > 0) {
    return lastSegment;
  }

  return label.name;
}

export function LabelChips({ labels, className }: LabelChipsProps) {
  const visibleLabels = (labels ?? []).filter((label) => label.name.trim().length > 0);

  if (visibleLabels.length === 0) {
    return null;
  }

  return (
    <span className={className ?? 'flex min-w-0 flex-wrap items-center gap-1'}>
      {visibleLabels.map((label) => {
        const fullName = label.name;

        return (
          <Tooltip key={label.id}>
            <TooltipTrigger asChild>
              <Badge
                variant="outline"
                title={fullName}
                aria-label={`Label ${fullName}`}
                className="max-w-32 truncate px-1.5 text-[0.65rem] font-medium"
              >
                {labelLeafText(label)}
              </Badge>
            </TooltipTrigger>
            <TooltipContent>{fullName}</TooltipContent>
          </Tooltip>
        );
      })}
    </span>
  );
}
