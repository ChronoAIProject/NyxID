import { test } from "@playwright/test";
import { accountBfcacheCases } from "./account-isolation.cases";

test.use({ browserName: "webkit" });
accountBfcacheCases();
