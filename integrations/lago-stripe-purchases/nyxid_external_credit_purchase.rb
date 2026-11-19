# SPDX-License-Identifier: AGPL-3.0-only

class NyxidExternalCreditPurchase < ApplicationRecord
  belongs_to :organization
  belongs_to :wallet
  belongs_to :wallet_transaction
end
