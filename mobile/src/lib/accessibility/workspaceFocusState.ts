type Control = { target: () => number | null; active: boolean; scope?: string };
type Origin = { scope: string; owner?: symbol; authority: number };

export function createWorkspaceFocusState(
  focus: (target: number) => void,
  schedule: (callback: () => void) => void = (callback) => callback(),
  authority: () => number = () => 0,
) {
  const controls = new Map<symbol, Control>();
  const modalOwners = new Map<
    symbol,
    { authority: number; focused: boolean; target?: () => number | null }
  >();
  const drawers = new Map<
    string,
    { target: () => number | null; focused: boolean }
  >();
  let admittedAuthority = authority();
  let preferred: symbol | undefined;
  let pending: Origin | undefined;
  let scope: string | undefined;
  let obscured = false;
  let revision = 0;
  function syncAuthority() {
    if (admittedAuthority === authority()) return;
    admittedAuthority = authority();
    controls.clear();
    drawers.clear();
    for (const modal of modalOwners.values()) modal.target = undefined;
    preferred = undefined;
    pending = undefined;
    revision++;
  }
  function restoreIfActive() {
    syncAuthority();
    if (!pending || obscured || modalOwners.size || pending.scope !== scope)
      return;
    const request = pending;
    const scheduledRevision = ++revision;
    schedule(() => {
      syncAuthority();
      if (
        scheduledRevision !== revision ||
        pending !== request ||
        obscured ||
        modalOwners.size ||
        scope !== request.scope
      )
        return;
      const original = request.owner ? controls.get(request.owner) : undefined;
      const control =
        original?.active && original.scope === scope
          ? original
          : [...controls.values()].find(
              (item) => item.active && item.scope === scope,
            );
      const target = control?.target();
      if (!target) return;
      pending = undefined;
      focus(target);
    });
  }
  return {
    setModalActive(owner: symbol, active: boolean) {
      syncAuthority();
      if (active) {
        if (!modalOwners.has(owner))
          modalOwners.set(owner, {
            authority: admittedAuthority,
            focused: false,
          });
        pending = undefined;
        revision++;
      } else {
        modalOwners.delete(owner);
      }
    },
    focusModalOnce(owner: symbol, target: () => number | null) {
      syncAuthority();
      const modal = modalOwners.get(owner);
      if (!modal || modal.focused || modal.authority !== admittedAuthority)
        return;
      modal.target = target;
      const scheduledRevision = revision;
      schedule(() => {
        syncAuthority();
        if (
          scheduledRevision !== revision ||
          modalOwners.get(owner) !== modal ||
          modal.focused
        )
          return;
        const current = modal.target?.();
        if (current) {
          modal.focused = true;
          focus(current);
        }
      });
    },
    context(nextScope: string | undefined, nextObscured: boolean) {
      syncAuthority();
      if (nextScope !== scope) {
        pending = undefined;
        preferred = undefined;
        revision++;
      }
      scope = nextScope;
      obscured = nextObscured;
      restoreIfActive();
    },
    register(
      owner: symbol,
      target: () => number | null,
      active: boolean,
      controlScope?: string,
    ) {
      syncAuthority();
      controls.set(owner, { target, active, scope: controlScope });
      restoreIfActive();
    },
    invoked(owner: symbol) {
      syncAuthority();
      if (controls.get(owner)?.active) preferred = owner;
    },
    release(owner: symbol) {
      controls.delete(owner);
      if (preferred === owner) preferred = undefined;
    },
    capture(originScope?: string): Origin | undefined {
      syncAuthority();
      if (!originScope) return;
      const owner =
        preferred && controls.get(preferred)?.scope === originScope
          ? preferred
          : [...controls.entries()].find(
              ([, control]) => control.scope === originScope,
            )?.[0];
      preferred = undefined;
      return { scope: originScope, owner, authority: admittedAuthority };
    },
    restore(origin: Origin | undefined) {
      syncAuthority();
      if (
        !origin ||
        modalOwners.size ||
        origin.scope !== scope ||
        origin.authority !== admittedAuthority
      )
        return;
      pending = origin;
      restoreIfActive();
    },
    focusDrawerOnce(key: string, target: () => number | null) {
      syncAuthority();
      if (modalOwners.size) return;
      const drawer = drawers.get(key) ?? { target, focused: false };
      if (drawer.focused) return;
      drawer.target = target;
      drawers.set(key, drawer);
      const scheduledRevision = revision;
      schedule(() => {
        syncAuthority();
        if (
          scheduledRevision !== revision ||
          modalOwners.size ||
          drawers.get(key) !== drawer ||
          drawer.focused
        )
          return;
        const current = drawer.target();
        if (current) {
          drawer.focused = true;
          focus(current);
        }
      });
    },
    releaseDrawer(key: string) {
      drawers.delete(key);
    },
  };
}
