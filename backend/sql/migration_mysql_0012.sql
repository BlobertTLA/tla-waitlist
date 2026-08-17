CREATE TABLE `active_war` (
  `id` BIGINT PRIMARY KEY NOT NULL,
  `etag` VARCHAR(128) NULL,
  `updated_at` BIGINT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE `war_participant` (
  `war_id` BIGINT NOT NULL,
  `entity_id` BIGINT NOT NULL,
  `category` ENUM('corporation', 'alliance') NOT NULL,
  PRIMARY KEY (`war_id`, `entity_id`, `category`),
  CONSTRAINT `war_participant_war` FOREIGN KEY (`war_id`) REFERENCES `active_war` (`id`) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE `war_meta` (
  `meta_key` VARCHAR(64) PRIMARY KEY NOT NULL,
  `meta_value` TEXT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
