# linux-bsec-exporter Ansible role

This Ansible role allows to setupt the linux-bsec-exporter automatically.

To use this role, add

```yaml
- src: https://github.com/jgosmann/linux-bsec-exporter.git
```

to your `requirements.yml` and then install the role with:

```bash
ansible-galaxy install -r requirements.yml
```

You can then use the role in your playbooks like so:

```yaml
- hosts: all
  tasks:
    - name: Provide linux-bsec-exporter config
      copy:
        src: linux-bsec-exporter/config.toml
        dest: /etc/linux-bsec-exporter/config.toml
        owner: root
        group: root
        mode: "0644"
      
- hosts: all
  roles:
    - role: linux-bsec-exporter
      vars:
        binary_path: /usr/local/bin/linux-bsec-exporter
```

Note that you have to provide the configuration file yourself.
This gives you the most flexibility
(copying a ready-made file, using a template, ...).

## Available role variables

* `binary_path` (string, default: `"linux-bsec-exporter"`):
  Path to the linux-bsec-exporter binary.
* `config_path` (string, default: `"/etc/linux-bsec-exporter/config.toml"`):
  Path to the configuration file (has to be provided indepent of the role).
