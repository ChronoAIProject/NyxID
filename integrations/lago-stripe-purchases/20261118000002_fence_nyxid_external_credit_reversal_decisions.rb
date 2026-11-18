# SPDX-License-Identifier: AGPL-3.0-only

class FenceNyxidExternalCreditReversalDecisions < ActiveRecord::Migration[8.0]
  disable_ddl_transaction!

  def up
    add_column :nyxid_external_credit_refunds, :decision_id, :uuid, if_not_exists: true
    add_index :nyxid_external_credit_refunds, :decision_id, unique: true, algorithm: :concurrently, if_not_exists: true
    add_index :nyxid_external_credit_refunds, :stripe_refund_id,
      unique: true, where: "wallet_transaction_id IS NOT NULL",
      name: "idx_nyxid_external_applied_reversals", algorithm: :concurrently, if_not_exists: true
    change_column_null :nyxid_external_credit_refunds, :wallet_transaction_id, true
    remove_index :nyxid_external_credit_refunds, :stripe_refund_id,
      name: "index_nyxid_external_credit_refunds_on_stripe_refund_id",
      algorithm: :concurrently, if_exists: true
  end

  def down
    raise ActiveRecord::IrreversibleMigration, "Retain the external reversal decisions"
  end
end
