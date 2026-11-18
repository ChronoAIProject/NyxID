# SPDX-License-Identifier: AGPL-3.0-only

class CreateNyxidExternalCreditRefunds < ActiveRecord::Migration[8.0]
  def change
    create_table :nyxid_external_credit_refunds, id: :uuid do |t|
      t.references :purchase, type: :uuid, null: false, foreign_key: {to_table: :nyxid_external_credit_purchases}
      t.references :wallet_transaction, type: :uuid, null: false, foreign_key: true, index: {unique: true}
      t.string :stripe_refund_id, null: false, index: {unique: true}
      t.bigint :credits, null: false
      t.bigint :amount_minor, null: false
      t.timestamps
    end
  end
end
