export interface ParticipantLike {
  name?: string | null;
  email: string;
}

export function formatParticipantName(participant: ParticipantLike) {
  return participant.name?.trim() || participant.email || 'Unknown';
}

export function formatParticipantEmail(participant: { email: string } | null) {
  return participant?.email.trim() || 'unknown sender';
}

export function formatParticipantList(participants: ParticipantLike[]) {
  if (participants.length === 0) {
    return 'Unknown';
  }

  return participants.map(formatParticipantName).join(', ');
}
