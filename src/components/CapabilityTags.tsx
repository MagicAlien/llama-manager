import { strings } from "@/lib/strings";
import type { CapabilityTag } from "@/lib/types";
import { Badge } from "@/components/ui/badge";

// T-037 — what the model can actually do, as one row of tags spanning the
// model card's width, above the model's own text.
//
// Which tags exist is decided by the backend from the model itself: the GGUF
// header answers Thinking / MTP / Tool use, the model's own file set answers
// Vision. This component renders what it was given and never guesses — a model
// that declares none of the four renders nothing at all (no empty row, no
// placeholder), and an unrecognised template leaves its tags absent rather
// than showing an "unknown" chip.
//
// The tags are facts about the model, not verdicts on it: they carry the
// informational variant, leaving pass/warn/destructive to the compatibility
// and availability badges, where those colours mean something.
const LABELS: Record<CapabilityTag, string> = {
  Thinking: strings.capabilities.thinking,
  Mtp: strings.capabilities.mtp,
  Vision: strings.capabilities.vision,
  ToolUse: strings.capabilities.toolUse,
};

export function CapabilityTags({ tags }: { tags: CapabilityTag[] }) {
  if (tags.length === 0) {
    return null;
  }

  return (
    <div className="flex w-full flex-wrap items-center gap-2">
      {tags.map((tag) => (
        <Badge key={tag} variant="info">
          {LABELS[tag]}
        </Badge>
      ))}
    </div>
  );
}
