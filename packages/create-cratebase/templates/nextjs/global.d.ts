// A side-effect CSS import (`import "./globals.css"` in app/layout.tsx)
// needs a module declaration for `tsc --noEmit` run standalone (outside
// `next build`'s own type-checking, which stubs this internally).
declare module "*.css";
