# SPDX-License-Identifier: AGPL-3.0-only

namespace :api do
  namespace :v1 do
    scope "wallets/:wallet_id/nyxid_external_credit_purchases" do
      get "capability", to: "nyxid_external_credit_purchases#capability"
      get ":purchase_id", to: "nyxid_external_credit_purchases#show"
      post ":purchase_id/refunds", to: "nyxid_external_credit_purchases#refund"
      post ":purchase_id/refunds/cancel", to: "nyxid_external_credit_purchases#cancel_refund",
        as: :cancel_nyxid_external_credit_reversal
      post "", to: "nyxid_external_credit_purchases#create"
    end
  end
end
