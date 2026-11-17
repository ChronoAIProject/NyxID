/**
 * Jest + React Native Testing Library for component behaviour that needs a
 * real React renderer (gesture, accessibility and layout props end to end).
 * The existing `node:test` suite (`pnpm test`) stays the fast default; this
 * runner only picks `*.jest.test.ts(x)` files so the two never overlap.
 */
const transformed = [
  "(jest-)?react-native",
  "@react-native(-community)?",
  "expo(nent)?",
  "@expo(nent)?/.*",
  "@expo-google-fonts/.*",
  "react-navigation",
  "@react-navigation/.*",
  "@sentry/react-native",
  "native-base",
  "react-native-svg",
  "lucide-react-native",
].join("|");

module.exports = {
  preset: "jest-expo",
  testMatch: [
    "<rootDir>/src/**/*.jest.test.ts",
    "<rootDir>/src/**/*.jest.test.tsx",
  ],
  setupFilesAfterEnv: ["<rootDir>/jest.setup.ts"],
  // pnpm nests packages under node_modules/.pnpm/<pkg>/node_modules/<pkg>;
  // keep transforming the same packages the jest-expo preset transforms.
  transformIgnorePatterns: [
    `node_modules/(?!(\\.pnpm/[^/]+/node_modules/)?(${transformed}))`,
  ],
};
