import {
  useArchiveThreadMutation,
  useBubbleUpMutation,
  useClassifyThreadMutation,
  useContact,
  useContactNoteMutation,
  useFeedView,
  useImboxView,
  useLoginMutation,
  useLogoutMutation,
  useMe,
  usePapertrailView,
  useReplyLaterThreadMutation,
  useScreenerDecisionMutation,
  useScreenerView,
  useSetAsideThreadMutation,
  useSetupState,
  useTrashThreadMutation,
} from './query';
import { queryKeys } from './queryKeys';

export function ApiHookSmoke() {
  void useMe();
  void useSetupState();
  void useScreenerView();
  void useImboxView();
  void useFeedView();
  void usePapertrailView();
  void useContact('person@example.com');
  void useLoginMutation();
  void useLogoutMutation();
  void useScreenerDecisionMutation();
  void useClassifyThreadMutation();
  void useArchiveThreadMutation();
  void useTrashThreadMutation();
  void useSetAsideThreadMutation();
  void useReplyLaterThreadMutation();
  void useContactNoteMutation();
  void useBubbleUpMutation();
  void queryKeys.contact('person@example.com');
  void queryKeys.view('imbox');

  return null;
}
