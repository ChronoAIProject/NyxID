const admin = db.getSiblingDB("admin");
if (!admin.auth(process.env.MONGO_INITDB_ROOT_USERNAME, process.env.MONGO_INITDB_ROOT_PASSWORD)) {
  throw new Error("MongoDB root authentication failed");
}
let status;
try {
  status = admin.runCommand({ replSetGetStatus: 1 });
} catch (error) {
  if (error.code !== 94) throw error;
  status = { code: 94 };
}
if (status.code === 94) {
  const result = admin.runCommand({ replSetInitiate: {
    _id: "nyxid-rs", members: [{ _id: 0, host: "mongodb:27017" }],
  } });
  if (!result.ok) throw new Error("Replica-set initialization failed");
} else if (!status.ok) {
  throw new Error("Could not inspect replica-set status");
} else if (status.set !== "nyxid-rs") {
  throw new Error("Existing replica-set name differs; coordinate configuration manually");
}
for (let attempt = 0; attempt < 60; attempt++) {
  if (admin.runCommand({ hello: 1 }).isWritablePrimary) quit(0);
  sleep(1000);
}
throw new Error("Replica set did not elect a primary");
