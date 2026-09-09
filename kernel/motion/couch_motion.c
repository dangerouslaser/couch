// SPDX-License-Identifier: GPL-2.0
/* HA100 MiraMEMS DA218B/MIR3DA acceleration, 1024 counts/g.
 * This board straps SD0 high: I2C2/0x27, despite stock DT listing 0x26.
 * Register map and 50Hz setting follow the vendor mir3da_core.c.
 * No interrupt pin is assumed and no factory calibration is written.
 */
#include <linux/module.h>
#include <linux/i2c.h>
#include <linux/kobject.h>
#include <linux/mutex.h>
#include <linux/delay.h>

static struct i2c_client *sensor;
static struct kobject *motion_kobj;
static DEFINE_MUTEX(motion_lock);
static u8 saved_range, saved_odr, saved_power;
static bool enabled;

static int read_regs(u8 reg, u8 *data, u16 count)
{
	struct i2c_msg msgs[2] = {
		{ .addr = 0x27, .len = 1, .buf = &reg },
		{ .addr = 0x27, .flags = I2C_M_RD, .len = count, .buf = data },
	};
	int ret;
#ifdef CONFIG_MTK_I2C_EXTENSION
	msgs[0].timing = msgs[1].timing = 100;
#endif
	ret = i2c_transfer(sensor->adapter, msgs, 2);
	return ret == 2 ? 0 : ret < 0 ? ret : -EIO;
}

static int write_reg(u8 reg, u8 value)
{
	u8 data[] = { reg, value };
	struct i2c_msg msg = { .addr = 0x27, .len = 2, .buf = data };
	int ret;
#ifdef CONFIG_MTK_I2C_EXTENSION
	msg.timing = 100;
#endif
	ret = i2c_transfer(sensor->adapter, &msg, 1);
	return ret == 1 ? 0 : ret < 0 ? ret : -EIO;
}

static void restore_registers(void)
{
	write_reg(0x11, saved_power | 0x80);
	write_reg(0x0f, saved_range);
	write_reg(0x10, saved_odr);
	write_reg(0x11, saved_power);
}

static ssize_t enabled_show(struct kobject *kobj, struct kobj_attribute *attr, char *buf)
{
	return scnprintf(buf, PAGE_SIZE, "%u\n", enabled);
}

static ssize_t enabled_store(struct kobject *kobj, struct kobj_attribute *attr,
			     const char *buf, size_t count)
{
	bool value;
	int ret;
	if (sysfs_streq(buf, "1")) value = true;
	else if (sysfs_streq(buf, "0")) value = false;
	else return -EINVAL;
	mutex_lock(&motion_lock);
	ret = write_reg(0x11, value ? saved_power & ~0x80 : saved_power | 0x80);
	if (!ret) {
		enabled = value;
		if (value)
			msleep(30); /* First conversion at 50Hz after standby. */
	}
	mutex_unlock(&motion_lock);
	return ret ? ret : count;
}

static ssize_t accel_show(struct kobject *kobj, struct kobj_attribute *attr, char *buf)
{
	u8 raw[6];
	int ret, xyz[3], i;
	mutex_lock(&motion_lock);
	ret = enabled ? read_regs(0x02, raw, sizeof(raw)) : -EAGAIN;
	mutex_unlock(&motion_lock);
	if (ret)
		return ret;
	for (i = 0; i < 3; i++)
		xyz[i] = (s16)((raw[2 * i + 1] << 8) | raw[2 * i]) >> 4;
	return scnprintf(buf, PAGE_SIZE, "%d %d %d\n", xyz[0], xyz[1], xyz[2]);
}

static struct kobj_attribute enabled_attr = __ATTR(enabled, 0600, enabled_show, enabled_store);
static struct kobj_attribute accel_attr = __ATTR(accel, 0400, accel_show, NULL);
static struct attribute *motion_attrs[] = { &enabled_attr.attr, &accel_attr.attr, NULL };
static const struct attribute_group motion_group = { .attrs = motion_attrs };

static int __init couch_motion_init(void)
{
	struct i2c_adapter *adapter = i2c_get_adapter(2);
	u8 id;
	int ret;
	if (!adapter)
		return -ENODEV;
	sensor = i2c_new_dummy(adapter, 0x27);
	i2c_put_adapter(adapter);
	if (!sensor)
		return -EBUSY;
	ret = read_regs(0x01, &id, 1);
	if (ret || id != 0x13) {
		ret = ret ? ret : -ENODEV;
		goto unregister;
	}
	ret = read_regs(0x0f, &saved_range, 1);
	if (ret) goto unregister;
	ret = read_regs(0x10, &saved_odr, 1);
	if (ret) goto unregister;
	ret = read_regs(0x11, &saved_power, 1);
	if (ret) goto unregister;
	ret = write_reg(0x11, saved_power | 0x80);
	if (ret) goto restore;
	ret = write_reg(0x0f, saved_range & ~0x03); /* +/-2g. */
	if (ret) goto restore;
	ret = write_reg(0x10, 0x06); /* All axes, vendor 50Hz ODR. */
	if (ret) goto restore;
	motion_kobj = kobject_create_and_add("couch_motion", kernel_kobj);
	if (!motion_kobj) { ret = -ENOMEM; goto restore; }
	ret = sysfs_create_group(motion_kobj, &motion_group);
	if (ret) { kobject_put(motion_kobj); goto restore; }
	pr_info("couch_motion: MIR3DA detected on I2C2/0x27; display-standby sampling available\n");
	return 0;
restore:
	restore_registers();
unregister:
	i2c_unregister_device(sensor);
	return ret;
}

static void __exit couch_motion_exit(void)
{
	sysfs_remove_group(motion_kobj, &motion_group);
	kobject_put(motion_kobj);
	restore_registers();
	i2c_unregister_device(sensor);
}
late_initcall(couch_motion_init);
module_exit(couch_motion_exit);
MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Couch HA100 MIR3DA display-standby motion sensor");
