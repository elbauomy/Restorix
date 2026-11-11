import type { ReactNode } from "react";

const S = ({ children, size = 16 }: { children: ReactNode; size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {children}
  </svg>
);

export const IconDisk = () => (
  <S>
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <circle cx="16.5" cy="15" r="1" />
    <path d="M6 9h8" />
  </S>
);
export const IconUsb = () => (
  <S>
    <rect x="7" y="9" width="10" height="12" rx="1.5" />
    <path d="M9 9V3h6v6M10.5 5.5h0M13.5 5.5h0" />
  </S>
);
export const IconImage = () => (
  <S>
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <circle cx="9" cy="10" r="2" />
    <path d="m21 16-5-5-8 9" />
  </S>
);
export const IconFolder = ({ open }: { open?: boolean }) => (
  <S size={15}>{open ? <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v1H7l-4 9V7Z M3 19l3.5-8H23l-3.5 8Z" /> : <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />}</S>
);
export const IconSearch = () => (
  <S>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </S>
);
export const IconPlay = () => (
  <S>
    <path d="M7 4v16l13-8Z" fill="currentColor" />
  </S>
);
export const IconStop = () => (
  <S>
    <rect x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor" />
  </S>
);
export const IconRefresh = () => (
  <S>
    <path d="M20 12a8 8 0 1 1-2.3-5.6M20 4v5h-5" />
  </S>
);
export const IconRestore = () => (
  <S>
    <path d="M12 3v12M7 10l5 5 5-5M4 19h16" />
  </S>
);
export const IconWarn = () => (
  <S>
    <path d="M12 3 2 20h20Z M12 10v4M12 17h0" />
  </S>
);
export const IconClose = () => (
  <S>
    <path d="M6 6l12 12M18 6 6 18" />
  </S>
);
export const IconChevron = ({ open }: { open?: boolean }) => (
  <S size={12}>
    <path d={open ? "m6 9 6 6 6-6" : "m9 6 6 6-6 6"} />
  </S>
);
export const IconFile = () => (
  <S size={15}>
    <path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z M14 3v6h6" />
  </S>
);
export const IconFilter = () => (
  <S>
    <path d="M3 5h18l-7 8v6l-4 2v-8Z" />
  </S>
);
export const IconExport = () => (
  <S>
    <path d="M12 15V3M7 8l5-5 5 5M4 15v4a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4" />
  </S>
);
