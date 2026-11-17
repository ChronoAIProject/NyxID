import { test } from "@playwright/test";
import { accountIsolationCases } from "./account-isolation.cases";

test.use({ browserName: "firefox" });
accountIsolationCases();
