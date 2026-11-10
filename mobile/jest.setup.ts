// Native modules the notification surfaces touch are replaced at the boundary.
jest.mock("react-native-reanimated", () => require("react-native-reanimated/mock"));
jest.mock("expo-notifications", () => ({
  setNotificationHandler: jest.fn(),
  addNotificationResponseReceivedListener: jest.fn(() => ({ remove: jest.fn() })),
  getLastNotificationResponseAsync: jest.fn(async () => null),
}));
