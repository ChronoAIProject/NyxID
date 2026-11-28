use super::*;
use crate::services::durable_wake_tests::resume_after;

#[derive(Clone, Copy)]
enum Edit {
    Backup,
    Restore,
    Companion,
    Catalog,
    Admin,
}

async fn at_each_transition(edit: Edit) {
    let control = connect_transaction_test_database("retire_matrix").await;
    with_history(&control,300,async{
        for boundary in 0..5 {
            let db=connect_transaction_test_database("retire_race").await;
            let mut f=fixture(&db).await;
            for _ in 0..boundary {
                let committed=step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease).await.unwrap();
                f.request.expected_step_revision=committed.revision;
            }
            let before=head(&db).await;
            let scope:Scope=field(&before,"scope").unwrap();
            let work=async {
                let result=step(&db,&f.config,&f.actor,&f.request,&CertifiedRelease).await;
                if matches!(edit,Edit::Admin){result.unwrap();}else{assert!(result.is_err());}
            };
            resume_after("retirement_claimed",work,async{
                match edit {
                    Edit::Backup|Edit::Restore=>{
                        let change=match edit {
                            Edit::Restore=>Change::AdvanceRestoreEpoch,
                            _=>Change::OpenDependency{inventory:Inventory::FullBackup,parents:Default::default(),formats:["v3".into()].into()},
                        };
                        append(&db,&f.config,&f.actor,&Append{recording:recording(f.request.expected_step_revision),scope,change}).await.unwrap();
                    }
                    Edit::Companion=>{
                        let mut tx=db.client().start_session().await.unwrap();
                        tx.start_transaction().await.unwrap();
                        invalidate_in_transaction(&db,&mut tx,"companion_removed").await.unwrap().unwrap();
                        db.collection::<Document>(crate::models::user::COLLECTION_NAME).delete_one(doc!{"_id":&f.actor.user_id}).session(&mut tx).await.unwrap();
                        transaction_service::commit(&mut tx).await.unwrap();
                    }
                    Edit::Catalog=>{
                        let admission=Admission{layout:doc!{"_id":SINGLETON,"scope":before.get("scope"),"proofs":before.get("proofs"),"restore_required":false},cohort:None};
                        let init=Initialization::begin(&db,Some(&admission)).await.unwrap();
                        db.collection::<Document>("saved_logins").create_index(IndexModel::builder().keys(doc!{"label":1}).build()).await.unwrap();
                        init.finish(&db).await.unwrap();
                    }
                    Edit::Admin=>{
                        let other=AuditActor::from_auth_user(&test_auth_user(&fresh()));
                        let other_request=Request{recording:recording(f.request.expected_step_revision),target:f.request.target.clone(),report_revision:f.request.report_revision,expected_step_revision:f.request.expected_step_revision};
                        assert!(step(&db,&f.config,&other,&other_request,&CertifiedRelease).await.is_err());
                    }
                }
            }).await;
            let after=head(&db).await;
            assert_eq!(after.contains_key("published_retirement"),boundary==4 && matches!(edit,Edit::Admin));
            if !after.contains_key("published_retirement") {
                let mut proposed=f.config.clone();proposed.encryption_key_previous=None;
                let proofs:Vec<proof::Proof>=field(&after,"proofs").unwrap();
                let scope:Scope=field(&after,"scope").unwrap();
                assert!(proof::verify(&scope.deployment_id,&proof::slots(&proposed).await.unwrap(),&proofs).await.is_err());
            }
            db.drop().await.unwrap();
        }
    }).await;
    control.drop().await.unwrap();
}

#[tokio::test]
async fn opening_backup_fences_every_transaction() {
    at_each_transition(Edit::Backup).await;
}
#[tokio::test]
async fn advancing_restore_epoch_fences_every_transaction() {
    at_each_transition(Edit::Restore).await;
}
#[tokio::test]
async fn startup_catalog_work_fences_every_transaction() {
    at_each_transition(Edit::Catalog).await;
}
#[tokio::test]
async fn second_admin_cannot_consume_another_decision() {
    at_each_transition(Edit::Admin).await;
}

#[tokio::test]
async fn exceptional_companion_delete_fences_every_transaction() {
    at_each_transition(Edit::Companion).await;
}
