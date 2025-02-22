use std::collections::hash_map::DefaultHasher;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

use anyhow::Result;
use aws_sdk_dynamodb::types::{AttributeValue, Put, TransactWriteItem, Update};
use aws_sdk_dynamodb::Client;

use serde::{de, Serialize};

use cqrs_es_example_domain::aggregate::{Aggregate, AggregateId};
use cqrs_es_example_domain::Event;

#[derive(Debug, Clone)]
pub struct EventPersistenceGateway {
  journal_table_name: String,
  journal_aid_index_name: String,
  snapshot_table_name: String,
  snapshot_aid_index_name: String,
  client: Client,
  shard_count: u64,
}

impl EventPersistenceGateway {
  pub fn new(
    client: Client,
    journal_table_name: String,
    journal_aid_index_name: String,
    snapshot_table_name: String,
    snapshot_aid_index_name: String,
    shard_count: u64,
  ) -> Self {
    Self {
      journal_table_name,
      journal_aid_index_name,
      snapshot_table_name,
      snapshot_aid_index_name,
      client,
      shard_count,
    }
  }

  pub async fn get_snapshot_by_id1<T>(&self, aid: String) -> Result<(T, usize, usize)>
  where
    T: for<'de> de::Deserialize<'de>,
  {
    let response = self
      .client
      .get_item()
      .table_name(self.snapshot_table_name.clone())
      .key("pkey", AttributeValue::S(aid))
      .send()
      .await?;
    let item = response.item().unwrap();
    let payload = item.get("payload").unwrap().as_s().unwrap();
    let aggregate = serde_json::from_str::<T>(payload).unwrap();
    let seq_nr = item.get("seq_nr").unwrap().as_n().unwrap().parse::<usize>().unwrap();
    let version = item.get("version").unwrap().as_n().unwrap().parse::<usize>().unwrap();
    Ok((aggregate, seq_nr, version))
  }

  pub async fn get_snapshot_by_id<T, AID: AggregateId>(&self, aid: &AID) -> Result<(T, usize, usize)>
  where
    T: for<'de> de::Deserialize<'de>,
  {
    let response = self
      .client
      .query()
      .table_name(self.snapshot_table_name.clone())
      .index_name(self.snapshot_aid_index_name.clone())
      .key_condition_expression("#aid = :aid AND #seq_nr > :seq_nr")
      .expression_attribute_names("#aid", "aid")
      .expression_attribute_names("#seq_nr", "seq_nr")
      .expression_attribute_values(":aid", AttributeValue::S(aid.to_string()))
      .expression_attribute_values(":seq_nr", AttributeValue::N(0.to_string()))
      .scan_index_forward(false)
      .limit(1)
      .send()
      .await?;

    if let Some(items) = response.items {
      if items.len() == 1 {
        let item = items[0].clone();
        let payload = item.get("payload").unwrap().as_s().unwrap();
        let aggregate = serde_json::from_str::<T>(payload).unwrap();
        let seq_nr = item.get("seq_nr").unwrap().as_n().unwrap().parse::<usize>().unwrap();
        let version = item.get("version").unwrap().as_n().unwrap().parse::<usize>().unwrap();
        return Ok((aggregate, seq_nr, version));
      } else {
        return Err(anyhow::anyhow!("No snapshot found for aggregate id: {}", aid));
      }
    } else {
      return Err(anyhow::anyhow!("No snapshot found for aggregate id: {}", aid));
    }
  }

  pub async fn get_events_by_id_and_seq_nr<T, AID: AggregateId>(&self, aid: &AID, seq_nr: usize) -> Result<Vec<T>>
  where
    T: Debug + for<'de> de::Deserialize<'de>,
  {
    let response = self
      .client
      .query()
      .table_name(self.journal_table_name.clone())
      .index_name(self.journal_aid_index_name.clone())
      .key_condition_expression("#aid = :aid AND #seq_nr > :seq_nr")
      .expression_attribute_names("#aid", "aid")
      .expression_attribute_values(":aid", AttributeValue::S(aid.to_string()))
      .expression_attribute_names("#seq_nr", "seq_nr")
      .expression_attribute_values(":seq_nr", AttributeValue::N(seq_nr.to_string()))
      .send()
      .await?;
    let mut events = Vec::new();
    if let Some(items) = response.items {
      for item in items {
        let payload = item.get("payload").unwrap();
        let str = payload.as_s().unwrap();
        let event = serde_json::from_str::<T>(str).unwrap();
        events.push(event);
      }
    }
    log::info!("epg: events = {:?}", events);
    Ok(events)
  }

  pub async fn store_event_with_snapshot_opt<A, E>(
    &mut self,
    event: &E,
    version: usize,
    aggregate: Option<&A>,
  ) -> Result<()>
  where
    A: ?Sized + Serialize + Aggregate,
    E: ?Sized + Serialize + Event,
  {
    // TODO: 最新のスナップショットを取得し別のskeyを付与して保存する
    // TODO: スナップショットの履歴が無限に増えないのように世代管理する
    match (event.is_created(), aggregate) {
      (true, Some(ar)) => {
        let _ = self
          .client
          .transact_write_items()
          .transact_items(TransactWriteItem::builder().put(self.put_snapshot(event, ar)?).build())
          .transact_items(TransactWriteItem::builder().put(self.put_journal(event)?).build())
          .send()
          .await?;
      }
      (true, None) => {
        panic!("Aggregate is not found");
      }
      (false, ar) => {
        let _ = self
          .client
          .transact_write_items()
          .transact_items(
            TransactWriteItem::builder()
              .update(self.update_snapshot(event, version, ar)?)
              .build(),
          )
          .transact_items(TransactWriteItem::builder().put(self.put_journal(event)?).build())
          .send()
          .await?;
      }
    }
    Ok(())
  }

  fn put_snapshot<E, A>(&mut self, event: &E, ar: &A) -> Result<Put>
  where
    A: ?Sized + Serialize + Aggregate,
    E: ?Sized + Serialize + Event,
  {
    let put_snapshot = Put::builder()
      .table_name(self.snapshot_table_name.clone())
      .item(
        "pkey",
        AttributeValue::S(self.resolve_pkey(event.aggregate_id(), self.shard_count)),
      )
      // ロックを取る場合は常にskey=resolve_skey(aid, 0)で行う
      .item("skey", AttributeValue::S(self.resolve_skey(event.aggregate_id(), 0)))
      .item("payload", AttributeValue::S(serde_json::to_string(ar)?))
      .item("aid", AttributeValue::S(event.aggregate_id().to_string()))
      .item("seq_nr", AttributeValue::N(ar.seq_nr().to_string()))
      .item("version", AttributeValue::N("1".to_string()))
      .condition_expression("attribute_not_exists(pkey) AND attribute_not_exists(skey)")
      .build();
    Ok(put_snapshot)
  }

  fn update_snapshot<E, A>(&mut self, event: &E, version: usize, ar_opt: Option<&A>) -> Result<Update>
  where
    A: ?Sized + Serialize + Aggregate,
    E: ?Sized + Serialize + Event,
  {
    let mut update_snapshot = Update::builder()
      .table_name(self.snapshot_table_name.clone())
      .update_expression("SET #version=:after_version")
      .key(
        "pkey",
        AttributeValue::S(self.resolve_pkey(event.aggregate_id(), self.shard_count)),
      )
      // ロックを取る場合は常にskey=resolve_skey(aid, 0)で行う
      .key("skey", AttributeValue::S(self.resolve_skey(event.aggregate_id(), 0)))
      .expression_attribute_names("#version", "version")
      .expression_attribute_values(":before_version", AttributeValue::N(version.to_string()))
      .expression_attribute_values(":after_version", AttributeValue::N((version + 1).to_string()))
      .condition_expression("#version=:before_version");
    if let Some(ar) = ar_opt {
      update_snapshot = update_snapshot
        .update_expression("SET #payload=:payload, #seq_nr=:seq_nr, #version=:after_version")
        .expression_attribute_names("#seq_nr", "seq_nr")
        .expression_attribute_names("#payload", "payload")
        .expression_attribute_values(":seq_nr", AttributeValue::N(ar.seq_nr().to_string()))
        .expression_attribute_values(":payload", AttributeValue::S(serde_json::to_string(ar)?));
    }
    Ok(update_snapshot.build())
  }

  fn resolve_pkey<AID: AggregateId>(&self, id: &AID, shard_count: u64) -> String {
    let mut hasher = DefaultHasher::new();
    id.to_string().hash(&mut hasher);
    let hash_value = hasher.finish();
    let remainder = hash_value % shard_count;
    format!("{}-{}", id.type_name(), remainder)
  }

  fn resolve_skey<AID: AggregateId>(&self, id: &AID, seq_nr: usize) -> String {
    format!("{}-{}-{}", id.type_name(), id.value(), seq_nr)
  }

  fn put_journal<E>(&mut self, event: &E) -> Result<Put>
  where
    E: ?Sized + Serialize + Event,
  {
    let pkey = self.resolve_pkey(event.aggregate_id(), self.shard_count);
    let skey = self.resolve_skey(event.aggregate_id(), event.seq_nr());
    let aid = event.aggregate_id().to_string();
    let seq_nr = event.seq_nr().to_string();
    let payload = serde_json::to_string(event)?;
    let occurred_at = event.occurred_at().timestamp_millis().to_string();

    // info!("pkey = {}", pkey);
    // info!("skey = {}", skey);
    // info!("aid = {}", aid);
    // info!("seq_nr = {}", seq_nr);
    // info!("payload = {}", payload);
    // info!("occurred_at = {}", occurred_at);

    let put_journal = Put::builder()
      .table_name(self.journal_table_name.clone())
      .item("pkey", AttributeValue::S(pkey))
      .item("skey", AttributeValue::S(skey))
      .item("aid", AttributeValue::S(aid))
      .item("seq_nr", AttributeValue::N(seq_nr))
      .item("payload", AttributeValue::S(payload))
      .item("occurred_at", AttributeValue::N(occurred_at))
      .build();

    Ok(put_journal)
  }
}
