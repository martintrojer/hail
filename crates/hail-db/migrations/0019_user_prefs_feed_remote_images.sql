-- Add a first-class privacy preference for Feed/newsletter remote images.
ALTER TABLE user_prefs
  ADD COLUMN feed_load_remote_images BOOLEAN NOT NULL DEFAULT 0;
