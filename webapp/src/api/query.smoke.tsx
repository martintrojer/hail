import {
  useArchiveThreadMutation,
  useBubbleUpMutation,
  useClassifyThreadMutation,
  useContact,
  useContactNoteMutation,
  useDeniedSenders,
  useFeedView,
  useImboxView,
  useArchiveView,
  useLoginMutation,
  useLogoutMutation,
  useMe,
  useNotSpamThreadMutation,
  usePapertrailView,
  useReplyLaterThreadMutation,
  useScreenerAllowedView,
  useScreenerDecisionMutation,
  useScreenerView,
  useUndoDenyMutation,
  useSetAsideThreadMutation,
  useSetupState,
  useSpamThreadMutation,
  useTrashThreadMutation,
  useTrashView,
} from './query';
import { queryKeys } from './queryKeys';

export function ApiHookSmoke() {
  void useMe();
  void useSetupState();
  void useScreenerView();
  void useScreenerAllowedView();
  void useDeniedSenders();
  void useImboxView();
  void useFeedView();
  void usePapertrailView();
  void useArchiveView();
  void useTrashView();
  void useContact('person@example.com');
  void useLoginMutation();
  void useLogoutMutation();
  void useScreenerDecisionMutation();
  void useUndoDenyMutation();
  void useClassifyThreadMutation();
  void useArchiveThreadMutation();
  void useTrashThreadMutation();
  void useSpamThreadMutation();
  void useNotSpamThreadMutation();
  void useSetAsideThreadMutation();
  void useReplyLaterThreadMutation();
  void useContactNoteMutation();
  void useBubbleUpMutation();
  void queryKeys.contact('person@example.com');
  void queryKeys.view('archive');
  void queryKeys.view('trash');
  void queryKeys.view('imbox');

  return null;
}
