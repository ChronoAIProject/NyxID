# SPDX-License-Identifier: AGPL-3.0-only

class NyxidExternalCreditRefund < ApplicationRecord
  belongs_to :purchase, class_name: "NyxidExternalCreditPurchase"
  belongs_to :wallet_transaction, optional: true
end
