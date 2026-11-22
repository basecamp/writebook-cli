# Seeds a Writebook for tests/live.rs: a published handbook with three leaves
# and a private book. Run with `bin/rails runner - < tests/support/seed.rb`.

Account.first || Account.create!(name: "On-call test")
user = User.find_by(email_address: "oncall@example.com") || User.create!(name: "Oncall", email_address: "oncall@example.com", password: "secret123456", role: :administrator)

pub = Book.find_by(title: "Ops & On-call Handbook") || Book.create!(title: "Ops & On-call Handbook", author: "37signals", published: true)
priv = Book.find_by(title: "Private Runbooks") || Book.create!(title: "Private Runbooks", author: "Ops", published: false)
[pub, priv].each { |b| b.update_access(editors: [user.id], readers: [user.id]) }
if pub.leaves.none?
  pub.press Section.new(body: "Everything about being on call."), title: "Getting started"
  pub.press Page.new(body: "# Deploys\n\nTo deploy, run `bin/deploy`. To roll back a deploy, run `bin/rollback`.\n"), title: "Deploys"
  pub.press Page.new(body: "# Queue backlog\n\nWhen the Resque queue grows, check the workers first.\n"), title: "Queue backlog & workers"
end
if priv.leaves.none?
  priv.press Page.new(body: "Secret database failover steps.\n"), title: "Database failover"
end
puts({pub: pub.id, priv: priv.id, leaves: pub.leaves.map { [_1.id, _1.title] }, priv_leaves: priv.leaves.map { [_1.id, _1.title] }}.inspect)
