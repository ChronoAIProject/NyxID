# SPDX-License-Identifier: AGPL-3.0-only

class CreateNyxidExternalCreditPurchases < ActiveRecord::Migration[8.0]
  def change
    create_table :nyxid_external_credit_purchases, id: :uuid do |t|
      t.references :organization, type: :uuid, null: false, foreign_key: true
      t.references :wallet, type: :uuid, null: false, foreign_key: true
      t.references :wallet_transaction, type: :uuid, null: false, foreign_key: true, index: {unique: true}
      t.string :purchase_id, null: false
      t.string :stripe_account_id, null: false
      t.string :stripe_customer_id, null: false
      t.string :stripe_checkout_id, null: false
      t.string :stripe_payment_intent_id, null: false
      t.string :currency, null: false
      t.bigint :credits, null: false
      t.bigint :subtotal_minor, null: false
      t.bigint :tax_minor, null: false
      t.bigint :total_minor, null: false
      t.timestamps
    end

    add_index :nyxid_external_credit_purchases, %i[organization_id purchase_id], unique: true
    add_index :nyxid_external_credit_purchases, %i[stripe_account_id stripe_payment_intent_id],
      unique: true, name: "idx_nyxid_purchase_stripe_payment"
    add_index :nyxid_external_credit_purchases, %i[stripe_account_id stripe_checkout_id],
      unique: true, name: "idx_nyxid_purchase_stripe_checkout"
  end
end
