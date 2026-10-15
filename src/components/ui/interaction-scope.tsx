import { createContext, useContext, type ReactNode } from "react";

const ActiveContext = createContext(true);

/** Retained pages must release portals and focus traps while off screen. */
export function InteractionScope({
  active,
  children,
}: {
  active: boolean;
  children: ReactNode;
}) {
  const parentActive = useContext(ActiveContext);
  return (
    <ActiveContext.Provider value={parentActive && active}>
      {children}
    </ActiveContext.Provider>
  );
}

export const useInteractionActive = () => useContext(ActiveContext);
