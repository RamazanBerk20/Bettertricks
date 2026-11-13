import "@testing-library/jest-dom/vitest";
import { configure } from "@testing-library/react";

// The first test in a file also pays for transforming and importing the whole app in jsdom;
// a 1 s `findBy*` budget is shorter than that cold start on a loaded machine.
configure({ asyncUtilTimeout: 10_000 });
